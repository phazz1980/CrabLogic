//! The control sheets: the controller board with its terminals, and under each terminal the
//! device it is bound to (a coil, a sensor, a button), supplied from the 24 V buses.
//!
//! One column per bound terminal, in the order of the panel. The sheet is full after 13 columns;
//! the next terminals go to the next sheet, which has the board and the buses again.

use std::collections::HashMap;

use crab_core::{
    part_key, split_part, symbol_key, Device, DeviceKind, Panel, PanelLayout, Project, COIL,
};

use crate::bus::{self, Elsewhere};
use crate::canvas::{Align, Canvas, DOT, INK, LOW, TEXT, TEXT_S, THICK, THIN, WEAK};
use crate::doc::{Page, Sheet};
use crate::lang::Lang;
use crate::nets::{aux_terminals, terminals_of, Nets, Wire, WireClass, BOARD, EXTERNAL};
use crate::route;
use crate::scene::{Dir, End, ItemKind, Port};
use crate::symbols::{draw_aux_field, draw_field_device, AUX_H, FIELD_PITCH, TABLE_LINE_H};
use crate::xref::{Xref, Xrefs};

/// Distance between the columns: the room of the widest device of a column.
const PITCH: f32 = FIELD_PITCH;
/// The board stands this far from the left and from the top of the body.
const BOARD_DX: f32 = 60.0;
const BOARD_DY: f32 = 80.0;
const BOARD_H: f32 = 80.0;
/// Top terminals of the devices, under the bottom of the board.
const DEVICE_DY: f32 = 80.0;
/// The plus bus, under the top terminals of the devices.
const PLUS_DY: f32 = 110.0;
/// The buses are two modules apart.
const BUS_GAP: f32 = 20.0;
/// Distance between the contacts that stand in series with a coil, above it.
const SERIES_STEP: f32 = 80.0;

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

/// The table of the contacts under the coil of `device`, one line each, in the order of the panel:
/// the main contacts with their terminals (`1-4 /2.3`, where the device stands on the power
/// sheets), then every auxiliary contact with its terminals (`13-14 /3.1`, where it is drawn).
/// A reference is left out where the contact is not drawn. Empty for a device without a coil.
pub(crate) fn contact_table(
    panel: &Panel,
    device: &Device,
    power: &Xrefs,
    coils: &Xrefs,
) -> Vec<String> {
    if !matches!(device.kind, DeviceKind::Contactor | DeviceKind::Relay) {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let poles: Vec<String> = terminals_of(panel, device)
        .into_iter()
        .filter(|t| t != "A1" && t != "A2")
        .collect();
    if let (Some(first), Some(last)) = (poles.first(), poles.last()) {
        let at = power
            .get(&device.id)
            .map_or_else(String::new, |at| format!(" {at}"));
        lines.push(format!("{first}-{last}{at}"));
    }
    for aux in panel.aux_of(&device.id) {
        let (near, far) = aux_terminals(panel, aux);
        let at = coils
            .get(&aux.id)
            .map_or_else(String::new, |at| format!(" {at}"));
        lines.push(format!("{near}-{far}{at}"));
    }
    lines
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
    let zone = page.zone(origin.0, origin.1 + 25.0);
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
    table: &[String],
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

/// The control sheets of the panel; none if no terminal of the board is bound to a device.
///
/// The first sheet is number `first_sheet`; `contacts` says where the contacts of the coils are
/// drawn on the power sheets, `coils` where the coils and the auxiliary contacts are drawn (the
/// references under the coils). Returns the sheets and the place of every device drawn.
pub fn control_sheets(
    project: &Project,
    nets: &Nets,
    lang: Lang,
    first_sheet: usize,
    contacts: &Xrefs,
    coils: &Xrefs,
    page: &Page,
) -> (Vec<Sheet>, Xrefs) {
    let board_x = page.x0() + BOARD_DX;
    let board_top = page.y0() + BOARD_DY;
    // Columns on a sheet.
    let capacity = (((page.x1() - board_x - 10.0) / PITCH) as usize).max(1);
    let panel = &project.panel;
    let layout = &project.panel_layout;
    let rows: Vec<(&str, &str, &Device)> = nets
        .board_terminals
        .iter()
        .filter_map(|t| {
            let d = panel.device(&t.device)?;
            Some((t.terminal.as_str(), t.tag.as_str(), d))
        })
        .collect();
    // With no terminal bound the sheet is there (with the board and the buses) only when devices
    // or wires were put on it by hand.
    if rows.is_empty() && layout.placed.is_empty() && panel.links.is_empty() {
        return (Vec::new(), Xrefs::new());
    }
    // The devices stand lower when contacts in series with a coil need room above it.
    let series_most = rows
        .iter()
        .map(|(_, _, d)| series_of(panel, d).len())
        .max()
        .unwrap_or(0);
    let device_top = board_top + BOARD_H + DEVICE_DY + series_most as f32 * SERIES_STEP;
    // The contact tables under the coils need room above the buses: the buses go down past the
    // four lines that fit already, so the last line keeps its margin above `+24V`.
    let table_lines = rows
        .iter()
        .map(|(_, _, d)| contact_table(panel, d, contacts, coils).len())
        .max()
        .unwrap_or(0);
    let table_shift = table_lines.saturating_sub(4) as f32 * TABLE_LINE_H;
    let plus_bus = device_top + PLUS_DY + table_shift;
    let minus_bus = plus_bus + BUS_GAP;
    let board = &project.config.board;
    let board_name = board.model.as_deref().unwrap_or(&board.fqbn);
    // Where the 24 V come from: the device that feeds the buses.
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

    let mut chunks: Vec<&[(&str, &str, &Device)]> = rows.chunks(capacity).collect();
    if chunks.is_empty() {
        chunks.push(&rows[..]);
    }
    let total = chunks.len();
    let mut sheets = Vec::new();
    let mut places = Xrefs::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let mut c = Canvas::new();
        c.use_labels(layout);
        c.set_sheet(&format!("c{index}"));
        let width = chunk.len().max(2) as f32 * PITCH;
        let board_bottom = board_top + BOARD_H;
        let board_item = c.begin(ItemKind::Symbol {
            view: "board",
            device: BOARD.to_owned(),
            origin: (board_x, board_top),
            ports: Vec::new(),
        });
        c.rect(board_x, board_top, width, BOARD_H, THICK, INK);
        c.bold(
            board_x + 8.0,
            board_top + 14.0,
            &format!("-{BOARD}"),
            TEXT,
            Align::Left,
            INK,
        );
        c.text(
            board_x + 50.0,
            board_top + 14.0,
            board_name,
            TEXT,
            Align::Left,
            WEAK,
        );
        c.end();

        let mut last_x = board_x;
        let mut pins: Pins = HashMap::new();
        for (i, &(terminal, tag, device)) in chunk.iter().enumerate() {
            let x = board_x + PITCH / 2.0 + i as f32 * PITCH;
            // The symbol of the device may have been moved by hand; it stays inside the frame.
            let (dx, dy) = layout.shift(&symbol_key("control", &device.id));
            let sx = (x + dx).clamp(page.x0() + 30.0, page.x1() - PITCH / 2.0);
            let sy = (device_top + dy).clamp(page.y0() + 20.0, page.y1() - 140.0);
            last_x = last_x.max(sx.max(x));
            c.enter(board_item);
            c.text(x, board_top - 14.0, tag, TEXT, Align::Center, INK);
            c.text(x, board_bottom - 14.0, terminal, TEXT, Align::Center, INK);
            c.circle(x, board_bottom, DOT, THIN, INK);
            c.end();
            c.port(board_item, port(terminal, (x, board_bottom), Dir::Down));
            pins.insert(
                (BOARD.to_owned(), terminal.to_owned()),
                ((x, board_bottom), Dir::Down),
            );

            let series = series_of(panel, device);
            let table = contact_table(panel, device, contacts, coils);
            let item = c.begin(ItemKind::Symbol {
                view: "control",
                device: device.id.clone(),
                origin: (sx, sy),
                ports: Vec::new(),
            });
            let field = if device.kind == DeviceKind::Aux {
                let (near, far) = aux_terminals(panel, device);
                let parent_at = contacts.get(&device.of).map(ToString::to_string);
                // The signal goes out at the top, the contact is fed from the bus below.
                Some(draw_aux_field(
                    &mut c,
                    &device.of,
                    device.form,
                    (sx, sy),
                    (&far, &near),
                    parent_at.as_deref(),
                ))
            } else {
                draw_field_device(&mut c, device, (sx, sy), &table)
            };
            c.end();
            let Some(field) = field else {
                continue;
            };
            c.port(item, port("signal", field.signal, Dir::Up));
            let terminal_of = |p: &str| terminal_of_port(panel, device, p);
            pins.insert(
                (device.id.clone(), terminal_of("signal")),
                (field.signal, Dir::Up),
            );
            if let Some(at) = field.plus {
                c.port(item, port("plus", at, Dir::Down));
                pins.insert((device.id.clone(), terminal_of("plus")), (at, Dir::Down));
            }
            if let Some(at) = field.minus {
                c.port(item, port("minus", at, Dir::Down));
                pins.insert((device.id.clone(), terminal_of("minus")), (at, Dir::Down));
            }
            places.insert(
                device.id.clone(),
                Xref {
                    sheet: first_sheet + index,
                    zone: page.zone(sx, sy + 25.0),
                },
            );
            // The signal wire from the terminal of the board: to the device, or through the
            // contacts that stand in series with a coil, one above the other.
            let coil_pt = (field.signal.0, field.signal.1 - DOT);
            let mut from_end = End::new(BOARD, terminal);
            let mut from_pt = (x, board_bottom + DOT);
            let mut from_dev: (String, Option<String>) =
                (BOARD.to_owned(), Some(terminal.to_owned()));
            for (j, contact) in series.iter().enumerate() {
                let (near, far) = aux_terminals(panel, contact);
                let (cdx, cdy) = layout.shift(&symbol_key("control", &contact.id));
                let cx = (x + cdx).clamp(page.x0() + 30.0, page.x1() - PITCH / 2.0);
                let cy = (sy - (series.len() - j) as f32 * SERIES_STEP + cdy).max(page.y0() + 20.0);
                let citem = c.begin(ItemKind::Symbol {
                    view: "control",
                    device: contact.id.clone(),
                    origin: (cx, cy),
                    ports: Vec::new(),
                });
                let parent_at = contacts.get(&contact.of).map(ToString::to_string);
                // The signal comes in at the top (the first terminal) and leaves at the bottom.
                draw_aux_field(
                    &mut c,
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
                        sheet: first_sheet + index,
                        zone: page.zone(cx, cy + 25.0),
                    },
                );
                let wire = wire_between(nets, (&from_dev.0, from_dev.1.as_deref()), &contact.id);
                draw_wire(
                    &mut c,
                    wire,
                    (from_end, End::new(&contact.id, "up")),
                    (from_pt, (cx, cy - DOT)),
                    layout,
                );
                from_end = End::new(&contact.id, "down");
                from_pt = (cx, cy + AUX_H + DOT);
                from_dev = (contact.id.clone(), Some(far));
            }
            let signal = if series.is_empty() {
                find_wire(nets, BOARD, terminal)
            } else {
                wire_between(nets, (&from_dev.0, from_dev.1.as_deref()), &device.id)
            };
            draw_wire(
                &mut c,
                signal,
                (from_end, End::new(&device.id, "signal")),
                (from_pt, coil_pt),
                layout,
            );
            // The supply wires down to the buses.
            let (plus, minus) = bus_terminals(panel, device);
            if let Some((px, py)) = field.plus {
                let wire = find_wire(nets, &device.id, &plus);
                if let Some(w) = wire {
                    c.begin(ItemKind::Wire {
                        key: w.key(),
                        from: End::new(&device.id, "plus"),
                        to: End::new(EXTERNAL, "plus"),
                    });
                }
                c.line(&[(px, py + DOT), (px, plus_bus)], THIN, LOW);
                c.dot(px, plus_bus, DOT, LOW);
                if let Some(number) = wire.map(|w| w.number) {
                    c.wire_number(
                        &wire.map_or(String::new(), |w| w.label_key()),
                        (px, (py + plus_bus) / 2.0),
                        &number.to_string(),
                        INK,
                    );
                }
                c.end();
            }
            if let Some((mx, my)) = field.minus {
                let wire = find_wire(nets, &device.id, &minus);
                if let Some(w) = wire {
                    c.begin(ItemKind::Wire {
                        key: w.key(),
                        from: End::new(&device.id, "minus"),
                        to: End::new(EXTERNAL, "minus"),
                    });
                }
                c.line(&[(mx, my + DOT), (mx, minus_bus)], THIN, LOW);
                c.dot(mx, minus_bus, DOT, LOW);
                if let Some(number) = wire.map(|w| w.number) {
                    // Between the device and the plus bus, which the wire crosses.
                    c.wire_number(
                        &wire.map_or(String::new(), |w| w.label_key()),
                        (mx, (my + plus_bus) / 2.0),
                        &number.to_string(),
                        INK,
                    );
                }
                c.end();
            }
        }

        // The devices put on the sheet by hand, then the wires drawn by hand between terminals.
        let drawn: Vec<&str> = rows.iter().map(|(_, _, d)| d.id.as_str()).collect();
        let refs = Refs {
            power: contacts,
            coils,
        };
        for (key, at) in &layout.placed {
            let (id, _) = split_part(key);
            let Some(device) = panel.device(id) else {
                continue;
            };
            let sheet = at.sheet.min(total - 1);
            let automatic = drawn.contains(&id)
                || (device.kind == DeviceKind::Aux && !device.series.is_empty());
            if at.power || sheet != index || automatic {
                continue;
            }
            let (dx, dy) = layout.shift(&symbol_key("control", key));
            let origin = (
                (at.x + dx).clamp(page.x0() + 10.0, page.x1() - 80.0),
                (at.y + dy).clamp(page.y0() + 30.0, page.y1() - 60.0),
            );
            draw_placed(&mut c, panel, key, origin, layout, &mut pins, refs);
            if let Some((name, xref)) = placed_xref(panel, key, origin, page, first_sheet + index) {
                places.insert(name, xref);
            }
        }
        for (lane, link) in panel.links.iter().enumerate() {
            let end = |s: &str| {
                let (device, terminal) = s.split_once(':')?;
                pins.get(&(device.to_owned(), terminal.to_owned())).copied()
            };
            let (Some(a), Some(b)) = (end(&link.from), end(&link.to)) else {
                continue;
            };
            let wire = nets.wires.iter().find(|w| w.key() == link.key());
            let (from_device, from_terminal) =
                link.from.split_once(':').unwrap_or((&link.from, ""));
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

        // The buses.
        let (from, to) = (page.x0() + 50.0, (last_x + 30.0).max(board_x + width));
        let sheet_of = |i: usize| Elsewhere {
            sheet: first_sheet + i,
        };
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
        for (y, name) in [(plus_bus, "+24V"), (minus_bus, "0V")] {
            bus::draw(&mut c, page, y, (from, to), name, &style, prev, next);
        }
        c.text(from, minus_bus + 30.0, &note, TEXT, Align::Left, WEAK);

        let title = lang.pick("Цепи управления", "Control circuits");
        sheets.push(Sheet::new(
            if total > 1 {
                format!("{title} ({}/{total})", index + 1)
            } else {
                title.to_owned()
            },
            c,
        ));
    }
    (sheets, places)
}
