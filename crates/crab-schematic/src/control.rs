//! The control sheets: the controller board with its terminals, and under each terminal the
//! device it is bound to (a coil, a sensor, a button), supplied from the 24 V buses.
//!
//! One column per bound terminal, in the order of the panel. The sheet is full after 13 columns;
//! the next terminals go to the next sheet, which has the board and the buses again.

use crab_core::{Device, DeviceKind, Project};

use crate::canvas::{Align, Canvas, DOT, INK, LOW, TEXT, THICK, THIN, WEAK};
use crate::doc::{Page, Sheet};
use crate::lang::Lang;
use crate::nets::{Nets, WireClass, BOARD, EXTERNAL};
use crate::symbols::{draw_field_device, FIELD_PITCH};
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

/// Number of the wire that ends at the terminal `terminal` of `device`.
fn wire_number(nets: &Nets, device: &str, terminal: &str) -> Option<u32> {
    nets.wires
        .iter()
        .find(|w| {
            (w.from.device == device && w.from.terminal == terminal)
                || (w.to.device == device && w.to.terminal == terminal)
        })
        .map(|w| w.number)
}

/// Terminals of a device that go to the buses: the `+24V` one and the `0V` one.
fn bus_terminals(kind: DeviceKind) -> (&'static str, &'static str) {
    match kind {
        DeviceKind::Contactor | DeviceKind::Relay => ("", "A2"),
        DeviceKind::Sensor => ("+", "-"),
        _ => ("1", ""),
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
    let device_top = board_top + BOARD_H + DEVICE_DY;
    let plus_bus = device_top + PLUS_DY;
    let minus_bus = plus_bus + BUS_GAP;
    // Columns on a sheet.
    let capacity = (((page.x1() - board_x - 10.0) / PITCH) as usize).max(1);
    let panel = &project.panel;
    let rows: Vec<(&str, &str, &Device)> = nets
        .board_terminals
        .iter()
        .filter_map(|t| {
            let d = panel.device(&t.device)?;
            Some((t.terminal.as_str(), t.tag.as_str(), d))
        })
        .collect();
    if rows.is_empty() {
        return (Vec::new(), Xrefs::new());
    }
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

    let chunks: Vec<&[(&str, &str, &Device)]> = rows.chunks(capacity).collect();
    let total = chunks.len();
    let mut sheets = Vec::new();
    let mut places = Xrefs::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let mut c = Canvas::new();
        let width = chunk.len() as f32 * PITCH;
        let board_bottom = board_top + BOARD_H;
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

        let mut last_x = board_x;
        for (i, &(terminal, tag, device)) in chunk.iter().enumerate() {
            let x = board_x + PITCH / 2.0 + i as f32 * PITCH;
            last_x = x;
            c.text(x, board_top - 14.0, tag, TEXT, Align::Center, INK);
            c.text(x, board_bottom - 14.0, terminal, TEXT, Align::Center, INK);
            c.circle(x, board_bottom, DOT, THIN, INK);

            let contacts_at = contacts.get(&device.id).map(ToString::to_string);
            let Some(field) =
                draw_field_device(&mut c, device, (x, device_top), contacts_at.as_deref())
            else {
                continue;
            };
            places.insert(
                device.id.clone(),
                Xref {
                    sheet: first_sheet + index,
                    zone: page.zone(x, device_top + 25.0),
                },
            );
            // The signal wire from the terminal of the board.
            c.line(
                &[(x, board_bottom + DOT), (x, field.signal.1 - DOT)],
                THIN,
                INK,
            );
            let signal = wire_number(nets, BOARD, terminal);
            if let Some(number) = signal {
                // Along the wire, left of it (7.1.3.5).
                c.vtext(
                    x - 8.0,
                    (board_bottom + device_top) / 2.0,
                    &number.to_string(),
                    TEXT,
                    Align::Center,
                    INK,
                );
            }
            // The supply wires down to the buses.
            let (plus, minus) = bus_terminals(device.kind);
            if let Some((px, py)) = field.plus {
                c.line(&[(px, py + DOT), (px, plus_bus)], THIN, LOW);
                c.dot(px, plus_bus, DOT, LOW);
                if let Some(number) = wire_number(nets, &device.id, plus) {
                    c.vtext(
                        px - 8.0,
                        (py + plus_bus) / 2.0,
                        &number.to_string(),
                        TEXT,
                        Align::Center,
                        INK,
                    );
                }
            }
            if let Some((mx, my)) = field.minus {
                c.line(&[(mx, my + DOT), (mx, minus_bus)], THIN, LOW);
                c.dot(mx, minus_bus, DOT, LOW);
                if let Some(number) = wire_number(nets, &device.id, minus) {
                    // Between the device and the plus bus, which the wire crosses.
                    c.vtext(
                        mx - 8.0,
                        (my + plus_bus) / 2.0,
                        &number.to_string(),
                        TEXT,
                        Align::Center,
                        INK,
                    );
                }
            }
        }

        // The buses.
        let (from, to) = (page.x0() + 50.0, last_x + 30.0);
        c.line(&[(from, plus_bus), (to, plus_bus)], THIN, LOW);
        c.line(&[(from, minus_bus), (to, minus_bus)], THIN, LOW);
        c.bold(page.x0() + 2.0, plus_bus, "+24V", TEXT, Align::Left, LOW);
        c.bold(page.x0() + 2.0, minus_bus, "0V", TEXT, Align::Left, LOW);
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
