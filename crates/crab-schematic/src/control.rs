//! The control sheets: a ladder between two rails, the `+24V` rail at the top and the `0V` rail at
//! the bottom, as in the EPLAN sample (`specs/panel-free-sheets`).
//!
//! Every used terminal of the board is a channel box of the board (`-A1:IO5`) that stands where
//! its circuit uses it: an output above its coil, an input under its contact or button and above
//! the `0V` rail. One column per terminal; the columns of one load are next to each other, and a
//! column name (the tag) stands under the rails. The contact table of a coil stands under the
//! `0V` rail, under its coil.
//!
//! A sheet holds 13 columns at most; a group of columns that does not fit into the rest of a sheet
//! starts the next one.

use std::collections::{HashMap, HashSet};

use crab_core::panel_layout::label_key;
use crab_core::{
    part_key, split_part, symbol_key, ContactForm, Device, DeviceKind, Panel, PanelLayout, Project,
    SheetId, COIL, MAX_SHEETS,
};

use crate::bus::{self, Elsewhere, TAIL};
use crate::canvas::{Align, Canvas, DOT, INK, LOW, TEXT, TEXT_S, THIN, WEAK};
use crate::doc::{Page, Sheet};
use crate::lang::Lang;
use crate::nets::{aux_terminals, terminals_of, Nets, Wire, WireClass, BOARD, EXTERNAL};
use crate::route;
use crate::scene::{Dir, End, ItemKind, Port};
use crate::symbols::{
    draw_aux_field, draw_channel, draw_field_device, draw_table, Field, AUX_H, CHANNEL_H,
    FIELD_PITCH, SENSOR_SIGNAL_DX,
};
use crate::xref::{Xref, Xrefs};

/// Distance between the columns: the room of the widest device of a column.
const PITCH: f32 = FIELD_PITCH;
/// The columns start this far right of the left edge of the body.
const COLUMN_DX: f32 = 60.0;
/// Room kept right of the last column, before the right edge of the body.
const COLUMN_RIGHT_ROOM: f32 = 10.0;
/// The `+24V` rail stands this far below the top of the body.
const PLUS_RAIL_DY: f32 = 60.0;
/// The rails start this far right of the left edge of the body (room for the reference of the
/// previous sheet).
const RAIL_DX: f32 = 50.0;
/// The `0V` rail stands this far above the bottom of the body: room under it for the contact
/// tables and for the captions of the columns.
const ZERO_RAIL_DY: f32 = 180.0;
/// The top of an output channel stands this far under the `+24V` rail; it is not joined to it.
const OUT_CHANNEL_DY: f32 = 30.0;
/// The top of an input channel stands this far above the `0V` rail; it is not joined to it.
const IN_CHANNEL_DY: f32 = 50.0;
/// A contact or a button on an input stands this far under the `+24V` rail.
const INPUT_DY: f32 = 40.0;
/// Height of a coil, from its `A1` to its `A2` terminal.
const COIL_H: f32 = 50.0;
/// The `A2` terminal of a coil stands this far above the `0V` rail.
const COIL_FOOT: f32 = 20.0;
/// The contact table under a coil starts this far under the `0V` rail.
const TABLE_DY: f32 = 16.0;
/// The table of a coil reaches this far left of the coil: the references of its rows (`/12.3`)
/// stand left of the contacts.
const TABLE_LEFT_ROOM: f32 = 84.0;
/// Distance between the contacts that stand in series with a coil, above it (at most; fewer
/// contacts take less room, more of them are packed closer, see `series_step`).
const SERIES_STEP: f32 = 80.0;
/// A contact in series keeps this far from the channel above it and from the coil below it.
const SERIES_MARGIN: f32 = 10.0;
/// A device stays this far inside the left edge of the body, and this far under its top edge.
const LEFT_MARGIN: f32 = 30.0;
const TOP_MARGIN: f32 = 20.0;
/// A device stays this far above the bottom edge of the body: room for the captions of the columns.
const BOTTOM_ROOM: f32 = 140.0;
/// Half the height of a device of the control sheet (a sensor, a contact, a coil): its reference
/// stands at its middle.
const HALF_FIELD_H: f32 = 25.0;
/// A device put on the sheet by hand stays inside the body: this far from the left and the top
/// edge, and this far from the right edge and the bottom edge (room for its references).
const HAND_LEFT: f32 = 10.0;
const HAND_TOP: f32 = 30.0;
const HAND_RIGHT_ROOM: f32 = 80.0;
const HAND_BOTTOM_ROOM: f32 = 60.0;
/// The most rows of a contact table under a coil; the last line is `+N` when there are more (the
/// rows shown are one less).
const MAX_TABLE_LINES: usize = 8;
/// The note under the `0V` rail stands this far under the rail, its lines this far apart.
const NOTE_DY: f32 = 18.0;
/// Room kept between the note under the `0V` rail and the table of the first column.
const NOTE_GAP: f32 = 12.0;
/// Width of a character of the note, about half its size (for the room it takes).
const NOTE_CHAR_W: f32 = 0.5 * TEXT_S;
/// The last line of a column caption stands this far above the bottom of the body.
const CAPTION_DY: f32 = 30.0;
/// Pitch of the lines of a column caption and of the note under the `0V` rail.
const CAPTION_LINE_H: f32 = 14.0;
/// Longest tag on one line of a column caption; a longer one is wrapped to two lines.
const CAPTION_CHARS: usize = 14;
/// A load is looked for at most this many devices along the `load` links.
const LOAD_STEPS: usize = 10;
/// The note under the `0V` rail stands this far right of the left edge of the body.
const NOTE_DX: f32 = 12.0;

/// The wire that ends at the terminal `terminal` of `device`.
fn find_wire<'a>(nets: &'a Nets, device: &str, terminal: &str) -> Option<&'a Wire> {
    nets.wires.iter().find(|w| {
        !nets.manual.contains(&w.key())
            && ((w.from.device == device && w.from.terminal == terminal)
                || (w.to.device == device && w.to.terminal == terminal))
    })
}

fn port(name: &str, (x, y): (f32, f32), dir: Dir) -> Port {
    Port {
        name: name.to_owned(),
        x,
        y,
        dir,
    }
}

/// Terminals of a device that go to the buses: the `+24V` one and the `0V` one.
fn bus_terminals(panel: &Panel, device: &Device) -> (String, String) {
    let (plus, minus) = match device.kind {
        DeviceKind::Contactor | DeviceKind::Relay => (String::new(), "A2".to_owned()),
        DeviceKind::Sensor => ("+".to_owned(), "-".to_owned()),
        // The contact on an input is fed through its first terminal, like a button.
        DeviceKind::Aux => (aux_terminals(panel, device).0, String::new()),
        _ => ("1".to_owned(), String::new()),
    };
    (plus, minus)
}

/// The terminal of `device` that a port of its symbol on the control sheet stands for (the ports
/// of the devices of the columns are named by place, those of the board and of the devices put on
/// the sheet by hand by terminal).
pub fn terminal_of_port(panel: &Panel, device: &Device, port: &str) -> String {
    let (near, far) = aux_terminals(panel, device);
    match (device.kind, port) {
        (DeviceKind::Contactor | DeviceKind::Relay, "signal") => "A1".to_owned(),
        (DeviceKind::Contactor | DeviceKind::Relay, "minus") => "A2".to_owned(),
        (DeviceKind::Sensor, "signal") => "S".to_owned(),
        (DeviceKind::Sensor, "plus") => "+".to_owned(),
        (DeviceKind::Sensor, "minus") => "-".to_owned(),
        (DeviceKind::Button, "signal") => "2".to_owned(),
        (DeviceKind::Button, "plus") => "1".to_owned(),
        (DeviceKind::Aux, "signal" | "down") => far,
        (DeviceKind::Aux, "plus" | "up") => near,
        _ => port.to_owned(),
    }
}

/// Where the terminals of the sheet are: the place and the direction a wire leaves in.
pub(crate) type Pins = HashMap<(String, String), ((f32, f32), Dir)>;

/// Draws a device put on the sheet by hand: a box with the designation and its terminals along
/// the top and the bottom edge, except the terminals in `leave_out` (drawn elsewhere, such as the
/// coil terminals of a contactor put on the sheet as its own coil symbol). Returns nothing; the
/// terminals are noted in `pins`.
pub(crate) fn draw_manual(
    c: &mut Canvas,
    panel: &Panel,
    device: &Device,
    (x, y): (f32, f32),
    leave_out: &[&str],
    pins: &mut Pins,
) {
    let terminals: Vec<String> = terminals_of(panel, device)
        .into_iter()
        .filter(|t| !leave_out.contains(&t.as_str()))
        .collect();
    let top_n = terminals.len().div_ceil(2);
    let bottom_n = terminals.len() - top_n;
    let (w, h) = (top_n.max(2) as f32 * 20.0 + 10.0, 40.0);
    let item = c.begin(ItemKind::Symbol {
        view: "control",
        device: device.id.clone(),
        origin: (x, y),
        ports: Vec::new(),
    });
    c.rect(x, y, w, h, THIN, INK);
    c.bold(
        x + w / 2.0,
        y + h / 2.0 + 5.0,
        &device.designation(),
        TEXT,
        Align::Center,
        INK,
    );
    if !device.rating.is_empty() {
        c.text(
            x + w / 2.0,
            y + h + 14.0,
            &device.rating,
            TEXT_S,
            Align::Center,
            WEAK,
        );
    }
    c.end();
    for (i, terminal) in terminals.iter().enumerate() {
        let (row, k, count) = if i < top_n {
            (0, i, top_n)
        } else {
            (1, i - top_n, bottom_n)
        };
        let step = w / count as f32;
        let px = x + step * (k as f32 + 0.5);
        let (py, dir, ty) = if row == 0 {
            (y, Dir::Up, y - 6.0)
        } else {
            (y + h, Dir::Down, y + h + 12.0)
        };
        c.circle(px, py, DOT, THIN, INK);
        c.text(px, ty, terminal, TEXT_S, Align::Center, INK);
        c.port(item, port(terminal, (px, py), dir));
        pins.insert((device.id.clone(), terminal.clone()), ((px, py), dir));
    }
}

/// The places of the devices on the other sheets, which the symbols drawn on a sheet write under
/// their devices (the contacts under a coil, the references of the hand-placed parts).
#[derive(Clone, Copy)]
pub(crate) struct Refs<'a> {
    /// Where the devices stand on the power sheets.
    pub power: &'a Xrefs,
    /// Where the coils and the auxiliary contacts are drawn.
    pub coils: &'a Xrefs,
}

/// The keys of the devices and of their parts that the generator draws on the control sheets: the
/// devices bound to a terminal of the board (with the coil of a contactor or relay, `K1/coil`),
/// and the auxiliary contacts in series with a coil. A hand placement of them is not drawn again:
/// the generator wins.
pub(crate) fn generated_on_control(panel: &Panel, nets: &Nets) -> HashSet<String> {
    let mut set = HashSet::new();
    for terminal in &nets.board_terminals {
        let Some(device) = panel.device(&terminal.device) else {
            continue;
        };
        set.insert(device.id.clone());
        if matches!(device.kind, DeviceKind::Contactor | DeviceKind::Relay) {
            set.insert(part_key(&device.id, COIL));
        }
    }
    set.extend(
        panel
            .devices
            .iter()
            .filter(|d| d.kind == DeviceKind::Aux && !d.series.is_empty())
            .map(|d| d.id.clone()),
    );
    set
}

/// What the hand-placed items of the drawing sheets are drawn with.
pub(crate) struct Hand<'a> {
    pub panel: &'a Panel,
    pub layout: &'a PanelLayout,
    pub nets: &'a Nets,
    pub page: &'a Page,
    pub refs: Refs<'a>,
    /// Keys (`K1`, `K1/coil`) of the devices and parts the generator draws on some sheet: a hand
    /// placement under such a key is not drawn.
    pub generated: &'a HashSet<String>,
}

/// Whether the item put on the sheet `at` is drawn on the sheet `on`: its own sheet, or, for a
/// sheet past the last one of its kind (`last` is the number of the last sheet of the kind the
/// generator makes), the last sheet of that kind. Free sheets are only drawn on their own.
fn drawn_on(at: SheetId, on: SheetId, last: usize) -> bool {
    match (at, on) {
        (SheetId::Free(k), SheetId::Free(n)) => k == n,
        (SheetId::Power(k), SheetId::Power(n)) | (SheetId::Control(k), SheetId::Control(n)) => {
            k == n || (n == last && k > last)
        }
        _ => false,
    }
}

/// Control sheets that the hand placements need when the generator draws none: up to the last
/// `Control(j)` that a device not drawn elsewhere is put on.
fn hand_control_count(project: &Project, generated: &HashSet<String>) -> usize {
    project
        .panel_layout
        .placed
        .iter()
        .filter(|(key, _)| {
            project.panel.device(split_part(key).0).is_some() && !generated.contains(*key)
        })
        .filter_map(|(_, at)| match at.sheet {
            SheetId::Control(j) => Some(j.saturating_add(1)),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .min(MAX_SHEETS)
}

/// Draws what is put by hand on the sheet `on` (see `drawn_on`; `last` is the number of the last
/// sheet of its kind): the devices and the coil parts, then the wires drawn by hand between the
/// terminals of the sheet. `sheet` is the document number of the sheet. `pins` holds the
/// terminals drawn on the sheet so far (the generator's ones too, where it draws them); the
/// coils and auxiliary contacts put here are added to `found` under the names the other sheets
/// refer to them by.
pub(crate) fn draw_hand(
    c: &mut Canvas,
    hand: &Hand,
    on: SheetId,
    last: usize,
    sheet: usize,
    pins: &mut Pins,
    found: &mut Xrefs,
) {
    let (panel, layout, page) = (hand.panel, hand.layout, hand.page);
    for (key, at) in &layout.placed {
        let (id, _) = split_part(key);
        if panel.device(id).is_none()
            || !drawn_on(at.sheet, on, last)
            || hand.generated.contains(key)
        {
            continue;
        }
        let (dx, dy) = layout.shift(&symbol_key("control", key));
        let origin = (
            (at.x + dx).clamp(page.x0() + HAND_LEFT, page.x1() - HAND_RIGHT_ROOM),
            (at.y + dy).clamp(page.y0() + HAND_TOP, page.y1() - HAND_BOTTOM_ROOM),
        );
        draw_placed(c, panel, key, origin, layout, pins, hand.refs);
        if let Some((name, xref)) = placed_xref(panel, key, origin, page, sheet) {
            found.insert(name, xref);
        }
    }
    draw_links(c, panel, hand.nets, pins);
}

/// Draws the wires drawn by hand between terminals (`panel.links`) whose two ends are in `pins`,
/// each with its number.
pub(crate) fn draw_links(c: &mut Canvas, panel: &Panel, nets: &Nets, pins: &Pins) {
    for (lane, link) in panel.links.iter().enumerate() {
        let end = |s: &str| {
            let (device, terminal) = s.split_once(':')?;
            pins.get(&(device.to_owned(), terminal.to_owned())).copied()
        };
        let (Some(a), Some(b)) = (end(&link.from), end(&link.to)) else {
            continue;
        };
        let wire = nets.wires.iter().find(|w| w.key() == link.key());
        let (from_device, from_terminal) = link.from.split_once(':').unwrap_or((&link.from, ""));
        let (to_device, to_terminal) = link.to.split_once(':').unwrap_or((&link.to, ""));
        c.begin(ItemKind::Wire {
            key: link.key(),
            from: End::new(from_device, from_terminal),
            to: End::new(to_device, to_terminal),
        });
        let points = link_route(a, b, (lane % 5) as f32 * 5.0);
        c.line(&points, THIN, INK);
        c.dot(a.0 .0, a.0 .1, DOT, INK);
        c.dot(b.0 .0, b.0 .1, DOT, INK);
        if let Some(w) = wire {
            let mid = ((points[1].0 + points[2].0) / 2.0, points[1].1);
            c.wire_number(&w.label_key(), mid, &w.number.to_string(), INK);
        }
        c.end();
    }
}

/// One contact of the table under a coil: its two terminals, whether it is normally closed, and
/// the reference of the sheet where it is drawn (`/2.2`), if it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContactRow {
    pub near: String,
    pub far: String,
    pub closed: bool,
    pub at: Option<String>,
}

/// The table of the contacts under a coil: the rows, and how many contacts are not shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ContactTable {
    pub rows: Vec<ContactRow>,
    pub hidden: usize,
}

/// The table of the contacts under the coil of `device`, one row per contact, in the order of the
/// panel: the main contacts, one row per pole (`1`-`2`, `3`-`4`), where the device stands on the
/// power sheets, then every auxiliary contact with its terminals (`13`-`14`, `21`-`22` closed),
/// where it is drawn. A reference is left out where the contact is not drawn. Empty for a device
/// without a coil.
pub(crate) fn contact_table(
    panel: &Panel,
    device: &Device,
    power: &Xrefs,
    coils: &Xrefs,
) -> ContactTable {
    if !matches!(device.kind, DeviceKind::Contactor | DeviceKind::Relay) {
        return ContactTable::default();
    }
    let poles: Vec<String> = terminals_of(panel, device)
        .into_iter()
        .filter(|t| t != "A1" && t != "A2")
        .collect();
    let pole_at = power.get(&device.id).map(|at| at.to_string());
    let mut rows: Vec<ContactRow> = poles
        .chunks(2)
        .filter_map(|pair| match pair {
            [near, far] => Some(ContactRow {
                near: near.clone(),
                far: far.clone(),
                closed: false,
                at: pole_at.clone(),
            }),
            _ => None,
        })
        .collect();
    for aux in panel.aux_of(&device.id) {
        let (near, far) = aux_terminals(panel, aux);
        rows.push(ContactRow {
            near,
            far,
            closed: aux.form == ContactForm::Nc,
            at: coils.get(&aux.id).map(|at| at.to_string()),
        });
    }
    limit_table(rows)
}

/// Keeps the table within `MAX_TABLE_LINES` lines: when there are more rows, the first
/// `MAX_TABLE_LINES - 1` are kept and `hidden` counts the rest (drawn as the `+N` line).
fn limit_table(mut rows: Vec<ContactRow>) -> ContactTable {
    let mut hidden = 0;
    if rows.len() > MAX_TABLE_LINES {
        hidden = rows.len() - (MAX_TABLE_LINES - 1);
        rows.truncate(MAX_TABLE_LINES - 1);
    }
    ContactTable { rows, hidden }
}

/// The place of a hand-placed part that the other sheets refer to, with the name it is kept under:
/// the coil `K1/coil` of a contactor or relay (under the device, `K1`) and an auxiliary contact put
/// on the sheet by hand (under its own id). `origin` is where the part is drawn on the sheet
/// `sheet`. `None` for the other parts.
pub(crate) fn placed_xref(
    panel: &Panel,
    key: &str,
    origin: (f32, f32),
    page: &Page,
    sheet: usize,
) -> Option<(String, Xref)> {
    let (id, part) = split_part(key);
    let device = panel.device(id)?;
    let name = match (part, device.kind) {
        (Some(COIL), DeviceKind::Contactor | DeviceKind::Relay) => id,
        (None, DeviceKind::Aux) => id,
        _ => return None,
    };
    let zone = page.zone(origin.0, origin.1 + HALF_FIELD_H);
    Some((name.to_owned(), Xref { sheet, zone }))
}

/// Draws the coil of a contactor or relay put on the sheet by hand as its own symbol, under the
/// key of the part (`K1/coil`). Its terminals `A1` (top) and `A2` (bottom) are noted in `pins`
/// under the device. `table` is the contact table under the coil (`contact_table`).
pub(crate) fn draw_coil(
    c: &mut Canvas,
    device: &Device,
    key: &str,
    (x, top): (f32, f32),
    table: &ContactTable,
    pins: &mut Pins,
) {
    let item = c.begin(ItemKind::Symbol {
        view: "control",
        device: key.to_owned(),
        origin: (x, top),
        ports: Vec::new(),
    });
    draw_field_device(c, device, (x, top), table);
    c.port(item, port("A1", (x, top), Dir::Up));
    c.port(item, port("A2", (x, top + 50.0), Dir::Down));
    c.end();
    pins.insert((device.id.clone(), "A1".to_owned()), ((x, top), Dir::Up));
    pins.insert(
        (device.id.clone(), "A2".to_owned()),
        ((x, top + 50.0), Dir::Down),
    );
}

/// Draws one entry of the hand-placed layout on a sheet at `origin`: a device as a box with its
/// terminals (`draw_manual`), or the coil part `K1/coil` of a contactor or relay as its own symbol
/// (`draw_coil`). Other parts are not drawn. A device whose coil is put apart leaves the coil
/// terminals out of its box.
pub(crate) fn draw_placed(
    c: &mut Canvas,
    panel: &Panel,
    key: &str,
    origin: (f32, f32),
    layout: &PanelLayout,
    pins: &mut Pins,
    refs: Refs,
) {
    let (id, part) = split_part(key);
    let Some(device) = panel.device(id) else {
        return;
    };
    let coil_device = matches!(device.kind, DeviceKind::Contactor | DeviceKind::Relay);
    match part {
        None => {
            let coil_apart = coil_device && layout.placed.contains_key(&part_key(id, COIL));
            let leave_out: &[&str] = if coil_apart { &["A1", "A2"] } else { &[] };
            draw_manual(c, panel, device, origin, leave_out, pins);
        }
        Some(COIL) if coil_device => {
            let table = contact_table(panel, device, refs.power, refs.coils);
            draw_coil(c, device, key, origin, &table, pins);
        }
        Some(_) => {}
    }
}

/// The route of a wire drawn by hand between two terminals: out of each terminal in its
/// direction, along between them.
pub(crate) fn link_route(a: ((f32, f32), Dir), b: ((f32, f32), Dir), lane: f32) -> Vec<(f32, f32)> {
    let (((ax, ay), da), ((bx, by), db)) = (a, b);
    let level = match (da, db) {
        (Dir::Down, Dir::Down) => ay.max(by) + 14.0 + lane,
        (Dir::Up, Dir::Up) => ay.min(by) - 14.0 - lane,
        _ => (ay + by) / 2.0 + lane,
    };
    vec![(ax, ay), (ax, level), (bx, level), (bx, by)]
}

/// The contacts in series with the coil `coil`, in the order of the panel (the order of the
/// wires of the circuit).
fn series_of<'a>(panel: &'a Panel, coil: &Device) -> Vec<&'a Device> {
    if !matches!(coil.kind, DeviceKind::Contactor | DeviceKind::Relay) {
        return Vec::new();
    }
    panel
        .devices
        .iter()
        .filter(|c| {
            c.kind == DeviceKind::Aux && c.series == coil.id && panel.device(&c.of).is_some()
        })
        .collect()
}

/// The distance between the contacts in series with a coil: `SERIES_STEP`, or less when `n` of
/// them and one more step do not fit between the channel (its bottom at `chan_bottom`) and the coil
/// (its top at `coil_top`), so that they always stay between the two.
fn series_step(n: usize, chan_bottom: f32, coil_top: f32) -> f32 {
    let room = (coil_top - chan_bottom - SERIES_MARGIN).max(0.0);
    SERIES_STEP.min(room / (n + 1) as f32)
}

/// The wire of the circuit that leaves `from` (a device and, when given, its terminal) and
/// arrives at the device `to`.
fn wire_between<'a>(nets: &'a Nets, from: (&str, Option<&str>), to: &str) -> Option<&'a Wire> {
    nets.wires.iter().find(|w| {
        !nets.manual.contains(&w.key())
            && w.from.device == from.0
            && from.1.is_none_or(|t| w.from.terminal == t)
            && w.to.device == to
    })
}

/// Draws one wire of the circuit between two points, as an item, with its number.
#[allow(clippy::too_many_arguments)]
fn draw_wire(
    c: &mut Canvas,
    wire: Option<&Wire>,
    ends: (End, End),
    (from, to): ((f32, f32), (f32, f32)),
    layout: &crab_core::PanelLayout,
) {
    if let Some(w) = wire {
        c.begin(ItemKind::Wire {
            key: w.key(),
            from: ends.0,
            to: ends.1,
        });
    }
    let r = route::drop(from, to, wire.and_then(|w| layout.bend(&w.key())), 0.0);
    c.line(&r.points, THIN, INK);
    if let Some(w) = wire {
        // Along the wire, left of it (7.1.3.5).
        c.wire_number(
            &w.label_key(),
            (to.0, r.label_y),
            &w.number.to_string(),
            INK,
        );
        c.end();
    }
}

/// A row of the sheets: a terminal of the board, its tag and the device bound to it.
#[derive(Clone)]
struct Row<'a> {
    terminal: &'a str,
    tag: &'a str,
    device: &'a Device,
    /// The motor, load or supply the device serves; `None` for the general circuits.
    load: Option<String>,
}

/// The load a device serves: the motor, load or supply its circuit ends at. A contactor or a
/// relay follows its `load` links; a sensor measures its `of` device; an auxiliary contact serves
/// the load of the device it belongs to; a button serves no load.
fn load_of(panel: &Panel, device: &Device) -> Option<String> {
    let start = match device.kind {
        DeviceKind::Button => return None,
        DeviceKind::Sensor | DeviceKind::Aux => panel.device(&device.of)?,
        _ => device,
    };
    let mut cur = start;
    for _ in 0..LOAD_STEPS {
        if matches!(
            cur.kind,
            DeviceKind::Motor | DeviceKind::Load | DeviceKind::Psu
        ) {
            return Some(cur.id.clone());
        }
        cur = panel.device(&cur.load)?;
    }
    None
}

/// Place of a load in the panel: the rows are ordered by it, the rows without one come last.
fn load_rank(panel: &Panel, load: Option<&str>) -> usize {
    load.and_then(|id| panel.devices.iter().position(|d| d.id == id))
        .unwrap_or(usize::MAX)
}

/// The load all the rows of a sheet serve, if they serve one.
fn shared_load<'a>(rows: &'a [Row]) -> Option<&'a str> {
    let first = rows.first()?.load.as_deref()?;
    rows.iter()
        .all(|r| r.load.as_deref() == Some(first))
        .then_some(first)
}

/// The rows of the sheets, in order. A group of rows of one load (they are next to each other)
/// goes on the sheet with the rows before it when it fits into the columns left; otherwise it
/// starts a new sheet. A group longer than a sheet fills the sheets one after another.
fn pack_rows<'a>(rows: &[Row<'a>], capacity: usize) -> Vec<Vec<Row<'a>>> {
    let mut sheets = Vec::new();
    let mut current: Vec<Row<'a>> = Vec::new();
    for group in rows.chunk_by(|a, b| a.load == b.load) {
        if !current.is_empty() && current.len() + group.len() > capacity {
            sheets.push(std::mem::take(&mut current));
        }
        for row in group {
            if current.len() == capacity {
                sheets.push(std::mem::take(&mut current));
            }
            current.push(row.clone());
        }
    }
    if !current.is_empty() {
        sheets.push(current);
    }
    sheets
}

/// The lines of a column caption: the tag, wrapped at the last space that fits when it is longer
/// than `CAPTION_CHARS`.
fn caption_lines(tag: &str) -> Vec<String> {
    if tag.chars().count() <= CAPTION_CHARS {
        return vec![tag.to_owned()];
    }
    let cut = tag
        .chars()
        .take(CAPTION_CHARS + 1)
        .collect::<Vec<char>>()
        .iter()
        .rposition(|c| *c == ' ')
        .filter(|&i| i > 0)
        .unwrap_or(CAPTION_CHARS);
    let mut second: String = tag
        .chars()
        .skip(cut)
        .collect::<String>()
        .trim_start()
        .to_owned();
    if second.chars().count() > CAPTION_CHARS {
        // The rest is cut to the line with an ellipsis, so that the line is `CAPTION_CHARS` long.
        second = second.chars().take(CAPTION_CHARS - 1).collect::<String>() + "…";
    }
    vec![
        tag.chars()
            .take(cut)
            .collect::<String>()
            .trim_end()
            .to_owned(),
        second,
    ]
}

/// Where a device of the column at `x` stands: its x (moved by hand, kept inside the sheet) and
/// its shift in y.
fn placed(l: &Ladder, device: &Device, x: f32) -> (f32, f32) {
    let (dx, dy) = l.layout.shift(&symbol_key("control", &device.id));
    (
        (x + dx).clamp(l.page.x0() + LEFT_MARGIN, l.page.x1() - PITCH / 2.0),
        dy,
    )
}

/// What the columns of a sheet are drawn with: the rails, the document and the places.
struct Ladder<'a> {
    panel: &'a Panel,
    nets: &'a Nets,
    layout: &'a PanelLayout,
    page: &'a Page,
    /// Where the devices stand on the power sheets, and where the coils and contacts are drawn.
    contacts: &'a Xrefs,
    coils: &'a Xrefs,
    /// Y of the `+24V` rail and of the `0V` rail.
    plus: f32,
    zero: f32,
    /// The document number of the sheet.
    sheet: usize,
}

/// A supply wire from `from` straight to the rail at `rail` (`+24V` above the device, `0V` below
/// it), with its number, and the node on the rail.
fn supply_line(
    c: &mut Canvas,
    wire: Option<&Wire>,
    device: &str,
    name: &str,
    from: (f32, f32),
    rail: f32,
) {
    if let Some(w) = wire {
        c.begin(ItemKind::Wire {
            key: w.key(),
            from: End::new(device, name),
            to: End::new(EXTERNAL, name),
        });
    }
    c.line(&[from, (from.0, rail)], THIN, LOW);
    c.dot(from.0, rail, DOT, LOW);
    if let Some(w) = wire {
        c.wire_number(
            &w.label_key(),
            (from.0, (from.1 + rail) / 2.0),
            &w.number.to_string(),
            INK,
        );
    }
    c.end();
}

/// Where the note under the `0V` rail stands and how it is aligned: at the left of the body when
/// its text ends before `first_x` (the left edge of the table of the first column, under it),
/// else right-aligned at the right end of the rail.
fn note_place(page: &Page, lines: &[String], first_x: Option<f32>) -> (f32, Align) {
    let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let left = page.x0() + NOTE_DX;
    let end = left + longest as f32 * NOTE_CHAR_W;
    if first_x.is_none_or(|x| end + NOTE_GAP <= x) {
        (left, Align::Left)
    } else {
        (page.x1() - TAIL, Align::Right)
    }
}

/// The caption under a column: the tag, as a text that can be moved.
fn draw_caption(c: &mut Canvas, device: &Device, x: f32, last: f32, tag: &str) {
    c.begin_label(&label_key(&device.id, "column"));
    let lines = caption_lines(tag);
    let n = lines.len();
    for (i, line) in lines.iter().enumerate() {
        let y = last - n.saturating_sub(1).saturating_sub(i) as f32 * CAPTION_LINE_H;
        c.text(x, y, line, TEXT_S, Align::Center, INK);
    }
    c.end_label();
}

/// Draws a column of a device that is an output: the channel of its terminal stands under the
/// `+24V` rail (not joined to it), the contacts in series with the coil go down to it, and the
/// coil stands above the `0V` rail, with its table under the rail.
fn draw_output(c: &mut Canvas, l: &Ladder, row: &Row, x: f32, pins: &mut Pins, places: &mut Xrefs) {
    let (device, terminal) = (row.device, row.terminal);
    let (sx, dy) = placed(l, device, x);
    let chan_top = l.plus + OUT_CHANNEL_DY;
    let chan_bottom = chan_top + CHANNEL_H;
    let (term, dir) = draw_channel(c, terminal, row.tag, (x, chan_top), Dir::Down);
    pins.insert((BOARD.to_owned(), terminal.to_owned()), (term, dir));

    let sy = (l.zero - COIL_FOOT - COIL_H + dy)
        .clamp(l.page.y0() + TOP_MARGIN, l.page.y1() - BOTTOM_ROOM);
    let table = contact_table(l.panel, device, l.contacts, l.coils);
    let item = c.begin(ItemKind::Symbol {
        view: "control",
        device: device.id.clone(),
        origin: (sx, sy),
        ports: Vec::new(),
    });
    let field = draw_field_device(c, device, (sx, sy), &ContactTable::default());
    draw_table(c, sx, l.zero + TABLE_DY, &table);
    c.end();
    let Some(field) = field else {
        return;
    };
    let terminal_of = |p: &str| terminal_of_port(l.panel, device, p);
    c.port(item, port("signal", field.signal, Dir::Up));
    pins.insert(
        (device.id.clone(), terminal_of("signal")),
        (field.signal, Dir::Up),
    );
    places.insert(
        device.id.clone(),
        Xref {
            sheet: l.sheet,
            zone: l.page.zone(sx, sy + HALF_FIELD_H),
        },
    );

    // The signal: from the channel, through the contacts in series, down to the coil.
    let series = series_of(l.panel, device);
    let step = series_step(series.len(), chan_bottom, sy);
    let mut from_end = End::new(BOARD, terminal);
    let mut from_pt = (x, chan_bottom + DOT);
    let mut from_dev: (String, Option<String>) = (BOARD.to_owned(), Some(terminal.to_owned()));
    for (j, contact) in series.iter().enumerate() {
        let (near, far) = aux_terminals(l.panel, contact);
        let (cdx, cdy) = l.layout.shift(&symbol_key("control", &contact.id));
        let cx = (x + cdx).clamp(l.page.x0() + LEFT_MARGIN, l.page.x1() - PITCH / 2.0);
        let cy = (sy - (series.len() - j) as f32 * step + cdy).max(chan_bottom + SERIES_MARGIN);
        let citem = c.begin(ItemKind::Symbol {
            view: "control",
            device: contact.id.clone(),
            origin: (cx, cy),
            ports: Vec::new(),
        });
        let parent_at = l.contacts.get(&contact.of).map(ToString::to_string);
        // The signal comes in at the top (the first terminal) and leaves at the bottom.
        draw_aux_field(
            c,
            &contact.of,
            contact.form,
            (cx, cy),
            (&near, &far),
            parent_at.as_deref(),
        );
        c.end();
        c.port(citem, port("up", (cx, cy), Dir::Up));
        c.port(citem, port("down", (cx, cy + AUX_H), Dir::Down));
        pins.insert((contact.id.clone(), near.clone()), ((cx, cy), Dir::Up));
        pins.insert(
            (contact.id.clone(), far.clone()),
            ((cx, cy + AUX_H), Dir::Down),
        );
        places.insert(
            contact.id.clone(),
            Xref {
                sheet: l.sheet,
                zone: l.page.zone(cx, cy + HALF_FIELD_H),
            },
        );
        let wire = wire_between(l.nets, (&from_dev.0, from_dev.1.as_deref()), &contact.id);
        draw_wire(
            c,
            wire,
            (from_end, End::new(&contact.id, "up")),
            (from_pt, (cx, cy - DOT)),
            l.layout,
        );
        from_end = End::new(&contact.id, "down");
        from_pt = (cx, cy + AUX_H + DOT);
        from_dev = (contact.id.clone(), Some(far));
    }
    let signal = if series.is_empty() {
        find_wire(l.nets, BOARD, terminal)
    } else {
        wire_between(l.nets, (&from_dev.0, from_dev.1.as_deref()), &device.id)
    };
    draw_wire(
        c,
        signal,
        (from_end, End::new(&device.id, "signal")),
        (from_pt, (sx, sy - DOT)),
        l.layout,
    );

    // The `0V` wire from the foot of the coil down to the rail.
    let (_, minus_terminal) = bus_terminals(l.panel, device);
    if let Some((mx, my)) = field.minus {
        c.port(item, port("minus", (mx, my), Dir::Down));
        pins.insert(
            (device.id.clone(), terminal_of("minus")),
            ((mx, my), Dir::Down),
        );
        let wire = find_wire(l.nets, &device.id, &minus_terminal);
        supply_line(c, wire, &device.id, "minus", (mx, my + DOT), l.zero);
    }
}

/// Draws a column of a device that is an input: the contact, the button or the sensor stands
/// under the `+24V` rail (the sensor in the middle) and its signal goes down to the channel of its
/// terminal, which stands above the `0V` rail (not joined to it).
fn draw_input(c: &mut Canvas, l: &Ladder, row: &Row, x: f32, pins: &mut Pins, places: &mut Xrefs) {
    let (device, terminal) = (row.device, row.terminal);
    let (sx, dy) = placed(l, device, x);
    let sensor = device.kind == DeviceKind::Sensor;
    let sy = if sensor {
        // The sensor stands in the middle of the body.
        (l.plus + l.zero) / 2.0 - HALF_FIELD_H + dy
    } else {
        l.plus + INPUT_DY + dy
    };
    let sy = sy.clamp(l.page.y0() + TOP_MARGIN, l.page.y1() - BOTTOM_ROOM);
    // The channel is part of the board: it stays in its column when the device is moved.
    let chan_x = if sensor { x - SENSOR_SIGNAL_DX } else { x };
    let chan_top = l.zero - IN_CHANNEL_DY;
    let (term, dir) = draw_channel(c, terminal, row.tag, (chan_x, chan_top), Dir::Up);
    pins.insert((BOARD.to_owned(), terminal.to_owned()), (term, dir));

    let item = c.begin(ItemKind::Symbol {
        view: "control",
        device: device.id.clone(),
        origin: (sx, sy),
        ports: Vec::new(),
    });
    let field: Option<Field> = match device.kind {
        DeviceKind::Aux => {
            let (near, far) = aux_terminals(l.panel, device);
            let parent_at = l.contacts.get(&device.of).map(ToString::to_string);
            // The `+24V` terminal is the top one, the signal leaves at the bottom.
            draw_aux_field(
                c,
                &device.of,
                device.form,
                (sx, sy),
                (&near, &far),
                parent_at.as_deref(),
            );
            Some(Field {
                signal: (sx, sy + AUX_H),
                plus: Some((sx, sy)),
                minus: None,
            })
        }
        _ => draw_field_device(c, device, (sx, sy), &ContactTable::default()),
    };
    c.end();
    let Some(field) = field else {
        return;
    };
    let terminal_of = |p: &str| terminal_of_port(l.panel, device, p);
    c.port(item, port("signal", field.signal, Dir::Down));
    pins.insert(
        (device.id.clone(), terminal_of("signal")),
        (field.signal, Dir::Down),
    );
    places.insert(
        device.id.clone(),
        Xref {
            sheet: l.sheet,
            zone: l.page.zone(sx, sy + HALF_FIELD_H),
        },
    );

    // The signal wire: from the device down to the channel.
    let signal = find_wire(l.nets, BOARD, terminal);
    draw_wire(
        c,
        signal,
        (End::new(BOARD, terminal), End::new(&device.id, "signal")),
        (
            (field.signal.0, field.signal.1 + DOT),
            (chan_x, chan_top - DOT),
        ),
        l.layout,
    );

    // The supply wires: the `+24V` terminal up to the rail, the `0V` one down to the rail.
    let (plus_terminal, minus_terminal) = bus_terminals(l.panel, device);
    if let Some(p) = field.plus {
        c.port(item, port("plus", p, Dir::Up));
        pins.insert((device.id.clone(), terminal_of("plus")), (p, Dir::Up));
        let wire = find_wire(l.nets, &device.id, &plus_terminal);
        supply_line(c, wire, &device.id, "plus", (p.0, p.1 - DOT), l.plus);
    }
    if let Some(m) = field.minus {
        c.port(item, port("minus", m, Dir::Down));
        pins.insert((device.id.clone(), terminal_of("minus")), (m, Dir::Down));
        let wire = find_wire(l.nets, &device.id, &minus_terminal);
        supply_line(c, wire, &device.id, "minus", (m.0, m.1 + DOT), l.zero);
    }
}

/// Draws the column of `row` at `x`: an output or an input, with its channel and its wires.
fn draw_column(c: &mut Canvas, l: &Ladder, row: &Row, x: f32, pins: &mut Pins, places: &mut Xrefs) {
    if matches!(row.device.kind, DeviceKind::Contactor | DeviceKind::Relay) {
        draw_output(c, l, row, x, pins, places);
    } else {
        draw_input(c, l, row, x, pins, places);
    }
}

/// The control sheets of the panel. With no terminal of the board bound there are none, unless devices
/// are put on control sheets by hand: then the sheets they need have the rails only.
///
/// The first sheet has the document number `first`; `contacts` says where the contacts of the coils
/// are drawn on the power sheets, `coils` where the coils and the auxiliary contacts are drawn (the
/// references under the coils). Returns the sheets and the place of every device drawn.
pub fn control_sheets(
    project: &Project,
    nets: &Nets,
    lang: Lang,
    first: usize,
    contacts: &Xrefs,
    coils: &Xrefs,
    page: &Page,
) -> (Vec<Sheet>, Xrefs) {
    let panel = &project.panel;
    let layout = &project.panel_layout;
    let column_x0 = page.x0() + COLUMN_DX;
    let plus = page.y0() + PLUS_RAIL_DY;
    let zero = page.y1() - ZERO_RAIL_DY;
    // Columns on a sheet.
    let capacity = (((page.x1() - column_x0 - COLUMN_RIGHT_ROOM) / PITCH) as usize).max(1);
    let mut rows: Vec<Row> = nets
        .board_terminals
        .iter()
        .filter_map(|t| {
            let device = panel.device(&t.device)?;
            Some(Row {
                terminal: &t.terminal,
                tag: &t.tag,
                device,
                load: load_of(panel, device),
            })
        })
        .collect();
    // The rows of one load next to each other, in the order of the panel of the load.
    rows.sort_by_key(|r| load_rank(panel, r.load.as_deref()));
    // The generator's devices are drawn on the sheets; a hand placement of a device the generator
    // draws is not drawn again.
    let generated_here = generated_on_control(panel, nets);
    let sheet_rows: Vec<Vec<Row>> = if rows.is_empty() {
        vec![Vec::new(); hand_control_count(project, &generated_here)]
    } else {
        pack_rows(&rows, capacity)
    };
    if sheet_rows.is_empty() {
        return (Vec::new(), Xrefs::new());
    }
    // Where the 24 V come from: the device that feeds the rail.
    let source = nets
        .wires
        .iter()
        .find(|w| w.class == WireClass::Control && w.from.terminal == "+24V")
        .map(|w| w.from.device.as_str());
    let note = match source {
        None | Some(EXTERNAL) => lang
            .pick(
                "Питание 24 В: внешний источник",
                "24 V supply: external source",
            )
            .to_owned(),
        Some(device) => {
            let at = contacts
                .get(device)
                .map_or_else(String::new, |at| format!(" ({at})"));
            format!(
                "{} -{device}{at}",
                lang.pick("Питание 24 В:", "24 V supply:")
            )
        }
    };
    // The note is two lines: the name of the supply, then its source.
    let note_lines: Vec<String> = match note.split_once(": ") {
        Some((head, tail)) => vec![format!("{head}:"), tail.to_owned()],
        None => vec![note],
    };

    let total = sheet_rows.len();
    // Devices the generator draws here or on the power sheets: a hand placement of them is not drawn.
    let mut generated = generated_here;
    generated.extend(contacts.keys().cloned());
    let mut sheets = Vec::new();
    let mut places = Xrefs::new();
    for (index, chunk) in sheet_rows.iter().enumerate() {
        let id = SheetId::Control(index);
        let sheet_no = first + index;
        let mut c = Canvas::new();
        c.use_labels(layout);
        c.set_sheet(&format!("c{index}"));
        let ladder = Ladder {
            panel,
            nets,
            layout,
            page,
            contacts,
            coils,
            plus,
            zero,
            sheet: sheet_no,
        };
        let mut pins: Pins = HashMap::new();
        for (i, row) in chunk.iter().enumerate() {
            let x = column_x0 + PITCH / 2.0 + i as f32 * PITCH;
            draw_column(&mut c, &ladder, row, x, &mut pins, &mut places);
            draw_caption(&mut c, row.device, x, page.y1() - CAPTION_DY, row.tag);
        }

        // The devices put on the sheet by hand, then the wires drawn by hand between terminals.
        let hand = Hand {
            panel,
            layout,
            nets,
            page,
            refs: Refs {
                power: contacts,
                coils,
            },
            generated: &generated,
        };
        draw_hand(
            &mut c,
            &hand,
            id,
            total.saturating_sub(1),
            sheet_no,
            &mut pins,
            &mut places,
        );

        // The rails run from the left of the body to the right edge, with the references to the
        // neighbouring sheets at their ends.
        let (from, to) = (page.x0() + RAIL_DX, page.x1() - TAIL);
        let sheet_of = |i: usize| Elsewhere { sheet: first + i };
        let style = bus::Style {
            width: THIN,
            rgb: LOW,
            bold: true,
            riser: None,
        };
        let (prev, next) = (
            index.checked_sub(1).map(sheet_of),
            (index + 1 < total).then(|| sheet_of(index + 1)),
        );
        for (y, name) in [(plus, "+24V"), (zero, "0V")] {
            bus::draw(&mut c, page, y, (from, to), name, &style, prev, next);
        }
        // The note stands under the `0V` rail at the left, unless the table of the first column is
        // there (from its left edge); then at the right end of the rail.
        let first_x = chunk
            .first()
            .map(|row| placed(&ladder, row.device, column_x0 + PITCH / 2.0).0 - TABLE_LEFT_ROOM);
        let note_at = note_place(page, &note_lines, first_x);
        for (k, line) in note_lines.iter().enumerate() {
            c.text(
                note_at.0,
                zero + NOTE_DY + k as f32 * CAPTION_LINE_H,
                line,
                TEXT_S,
                note_at.1,
                WEAK,
            );
        }

        // The title: the load the columns serve, else the default one.
        let base = match shared_load(chunk) {
            Some(load) => format!("{} -{load}", lang.pick("Управление", "Control")),
            None => lang.pick("Цепи управления", "Control circuits").to_owned(),
        };
        let default = if total > 1 {
            format!("{base} ({}/{total})", index + 1)
        } else {
            base
        };
        let caption = layout.caption(id).map_or(default, str::to_owned);
        sheets.push(Sheet::new(Some(id), caption, c));
    }
    (sheets, places)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_second_caption_line_is_cut_with_an_ellipsis() {
        let lines = caption_lines("Датчик уровня верхнего бака резервный");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].chars().count(), CAPTION_CHARS, "{lines:?}");
        assert!(lines[1].ends_with('…'), "{lines:?}");
    }

    #[test]
    fn a_short_caption_is_one_line() {
        assert_eq!(caption_lines("K1"), vec!["K1".to_owned()]);
    }

    #[test]
    fn a_contact_table_keeps_eight_lines_and_counts_the_rest() {
        let row = |i: usize| ContactRow {
            near: format!("{i}1"),
            far: format!("{i}2"),
            closed: false,
            at: None,
        };
        let rows: Vec<ContactRow> = (0..10).map(row).collect();
        // Seven rows and the line `+3` make the eighth line.
        let shown = limit_table(rows.clone());
        assert_eq!(shown.rows.len(), MAX_TABLE_LINES - 1);
        assert_eq!(shown.hidden, 3);
        assert_eq!(shown.rows.last(), Some(&row(6)));
        let full = limit_table(rows[..MAX_TABLE_LINES].to_vec());
        assert_eq!((full.rows.len(), full.hidden), (MAX_TABLE_LINES, 0));
    }
}
