//! The control sheets: the controller board with its terminals, and under each terminal the
//! device it is bound to (a coil, a sensor, a button), supplied from the 24 V buses.
//!
//! One column per bound terminal, in the order of the panel. The sheet is full after 13 columns;
//! the next terminals go to the next sheet, which has the board and the buses again.

use std::collections::HashMap;

use crab_core::{symbol_key, Device, DeviceKind, Panel, Project};

use crate::bus::{self, Elsewhere};
use crate::canvas::{Align, Canvas, DOT, INK, LOW, TEXT, TEXT_S, THICK, THIN, WEAK};
use crate::doc::{Page, Sheet};
use crate::lang::Lang;
use crate::nets::{aux_terminals, terminals_of, Nets, Wire, WireClass, BOARD, EXTERNAL};
use crate::route;
use crate::scene::{Dir, End, ItemKind, Port};
use crate::symbols::{draw_aux_field, draw_field_device, AUX_H, FIELD_PITCH};
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
/// the top and the bottom edge. Returns nothing; the terminals are noted in `pins`.
pub(crate) fn draw_manual(
    c: &mut Canvas,
    panel: &Panel,
    device: &Device,
    (x, y): (f32, f32),
    pins: &mut Pins,
) {
    let terminals = terminals_of(panel, device);
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
/// drawn. Returns the sheets and the place of every device drawn.
pub fn control_sheets(
    project: &Project,
    nets: &Nets,
    lang: Lang,
    first_sheet: usize,
    contacts: &Xrefs,
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
    let plus_bus = device_top + PLUS_DY;
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
            let contacts_at = contacts.get(&device.id).map(ToString::to_string);
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
                draw_field_device(&mut c, device, (sx, sy), contacts_at.as_deref())
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
        for (id, at) in &layout.placed {
            let Some(device) = panel.device(id) else {
                continue;
            };
            let sheet = at.sheet.min(total - 1);
            let automatic = drawn.contains(&id.as_str())
                || (device.kind == DeviceKind::Aux && !device.series.is_empty());
            if at.power || sheet != index || automatic {
                continue;
            }
            let (dx, dy) = layout.shift(&symbol_key("control", id));
            let origin = (
                (at.x + dx).clamp(page.x0() + 10.0, page.x1() - 80.0),
                (at.y + dy).clamp(page.y0() + 30.0, page.y1() - 60.0),
            );
            draw_manual(&mut c, panel, device, origin, &mut pins);
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
