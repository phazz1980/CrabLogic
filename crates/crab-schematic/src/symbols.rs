//! Symbols of the panel devices, drawn after IEC 60617 (own drawings, no outside libraries).
//!
//! Every device is drawn as many-pole: one conductor per pole, `POLE` apart, the supply side at
//! the top and the load side at the bottom. All the sizes are multiples of half a module (5 units,
//! 1.25 mm), the poles and the terminals lie on the grid of GOST IEC 61082-1 7.1.2.2.
//! Movable parts are drawn as the standard asks (7.4.4): breakers and contacts open, the contact
//! moves to the right when its coil is energised.

use crab_core::{Device, DeviceKind, Supply};

use crate::canvas::{Align, Canvas, DOT, EARTH, INK, TEXT, TEXT_L, TEXT_XL, THICK, THIN, WEAK};
use crate::lang::Lang;

/// Distance between the conductors of a device: two modules, so that text fits between the wires.
pub const POLE: f32 = 20.0;
/// Room to the left of the first pole for the designation and the rating.
const LABEL_W: f32 = 70.0;
/// Room to the right of the last pole for a coil or a caption.
const TAIL_W: f32 = 70.0;
/// The tallest symbol; the rows of a sheet are at least this far apart.
pub const HEIGHT: f32 = 130.0;
/// Height of the contact symbol of a breaker, a contactor or a relay.
const CONTACT_H: f32 = 100.0;
/// Pitch of the lines of a rating.
const LINE_H: f32 = 18.0;

/// Width of the column of a device with `n` conductors.
pub fn column_width(n: usize) -> f32 {
    LABEL_W + n.saturating_sub(1) as f32 * POLE + TAIL_W
}

/// X of the first pole (and of the contacts) of a device whose column starts at `left`.
pub fn contacts_x(left: f32) -> f32 {
    left + LABEL_W
}

/// Where the wires meet a drawn device.
pub struct Geometry {
    /// X of the conductors, in the order of the poles.
    pub poles: Vec<f32>,
    /// Y of the supply side terminals.
    pub in_y: f32,
    /// Y of the load side terminals (devices that pass the conductors on).
    pub out_y: f32,
    /// Where the protective earth leaves the device, if it has a body to earth.
    pub pe: Option<(f32, f32)>,
}

/// The rating cut into short lines for the narrow room left of the poles: at the commas, and a
/// line that is still long at its last space (`0,75 кВт` → `0,75` / `кВт`). At most three lines.
fn rating_lines(rating: &str) -> Vec<String> {
    const SHORT: usize = 7;
    let mut lines = Vec::new();
    for part in rating.split(", ").filter(|p| !p.is_empty()) {
        match part.rsplit_once(' ') {
            Some((a, b)) if part.chars().count() > SHORT => {
                lines.push(a.to_owned());
                lines.push(b.to_owned());
            }
            _ => lines.push(part.to_owned()),
        }
    }
    lines.truncate(3);
    lines
}

fn circle_edge(cx: f32, cy: f32, r: f32, x: f32) -> f32 {
    cy - (r * r - (x - cx) * (x - cx)).max(0.0).sqrt()
}

/// Captions that depend on the rest of the document.
#[derive(Default)]
pub struct Notes<'a> {
    /// Name of the input or output a coil is bound to.
    pub tag: Option<&'a str>,
    /// Where the coil of a contactor or relay is drawn (`/2.C3`: sheet 2, zone C3).
    pub coil_at: Option<&'a str>,
}

/// Draws `device` with its column starting at `left` and its top terminals at `top`.
pub fn draw_device(
    c: &mut Canvas,
    device: &Device,
    (left, top): (f32, f32),
    n: usize,
    supply: Supply,
    lang: Lang,
    notes: &Notes,
) -> Geometry {
    let poles: Vec<f32> = (0..n).map(|k| left + LABEL_W + k as f32 * POLE).collect();
    let first = poles[0];
    let last = poles[n - 1];
    let centre = (first + last) / 2.0;

    // The designation is left of the symbol, the technical data under it (7.1.6.1, 7.1.2.4).
    c.bold(
        left + 2.0,
        top + 35.0,
        &device.designation(),
        TEXT,
        Align::Left,
        INK,
    );
    for (i, line) in rating_lines(&device.rating).iter().enumerate() {
        c.text(
            left + 2.0,
            top + 35.0 + (i + 1) as f32 * LINE_H,
            line,
            TEXT,
            Align::Left,
            WEAK,
        );
    }

    let mut geo = Geometry {
        poles: poles.clone(),
        in_y: top,
        out_y: top,
        pe: None,
    };
    let mid = top + CONTACT_H / 2.0;
    match device.kind {
        DeviceKind::Breaker | DeviceKind::Rcd | DeviceKind::Contactor | DeviceKind::Relay => {
            geo.out_y = top + CONTACT_H;
            for &x in &poles {
                c.circle(x, top, DOT, THIN, INK);
                c.circle(x, top + CONTACT_H, DOT, THIN, INK);
                c.line(&[(x, top + DOT), (x, top + 30.0)], THIN, INK);
                // The blade, open: it moves to the right to close (7.4.4.4).
                c.line(&[(x, top + 30.0), (x - 10.0, top + 70.0)], THIN, INK);
                c.line(&[(x, top + 70.0), (x, top + CONTACT_H - DOT)], THIN, INK);
                match device.kind {
                    DeviceKind::Contactor | DeviceKind::Relay => {
                        c.circle(x - 5.0, mid, 5.0, THIN, INK);
                    }
                    _ => {
                        // The cross of the breaker.
                        c.line(&[(x - 5.0, top + 15.0), (x + 5.0, top + 25.0)], THIN, INK);
                        c.line(&[(x - 5.0, top + 25.0), (x + 5.0, top + 15.0)], THIN, INK);
                    }
                }
            }
            // Mechanical link of the poles.
            if n > 1 {
                c.dashed(&[(first - 5.0, mid), (last - 5.0, mid)], THIN, INK);
            }
            match device.kind {
                DeviceKind::Rcd => {
                    // The toroid of the residual-current device around the conductors.
                    c.rect(
                        first - 10.0,
                        top + 5.0,
                        last - first + 20.0,
                        10.0,
                        THIN,
                        INK,
                    );
                    c.text(
                        last + 15.0,
                        mid,
                        lang.pick("УЗО", "RCD"),
                        TEXT,
                        Align::Left,
                        WEAK,
                    );
                }
                DeviceKind::Contactor | DeviceKind::Relay => {
                    // The coil, joined to the contacts by the mechanical link.
                    let x = last + 15.0;
                    c.dashed(&[(last - 5.0, mid), (x, mid)], THIN, INK);
                    c.rect(x, top + 30.0, 20.0, 40.0, THIN, INK);
                    if let Some(tag) = notes.tag {
                        c.text(x + 10.0, top + 85.0, tag, TEXT, Align::Center, WEAK);
                    }
                    if let Some(at) = notes.coil_at {
                        c.text(x + 10.0, top + 103.0, at, TEXT, Align::Center, INK);
                    }
                }
                _ => {}
            }
        }
        DeviceKind::Psu => {
            let (x0, x1) = (first - 20.0, last + 20.0);
            let (y0, y1) = (top + 20.0, top + 80.0);
            for &x in &poles {
                c.circle(x, top, DOT, THIN, INK);
                c.line(&[(x, top + DOT), (x, y0)], THIN, INK);
            }
            c.rect(x0, y0, x1 - x0, y1 - y0, THIN, INK);
            c.line(&[(x0, y1), (x1, y0)], THIN, INK);
            c.text(x0 + 12.0, y0 + 12.0, "~", TEXT_L, Align::Center, INK);
            c.text(x1 - 12.0, y1 - 12.0, "=", TEXT_L, Align::Center, INK);
            c.text(
                x1 + 5.0,
                (y0 + y1) / 2.0,
                lang.pick("24 В", "24 V"),
                TEXT,
                Align::Left,
                WEAK,
            );
            geo.out_y = y1;
            geo.pe = Some((centre, y1));
        }
        DeviceKind::Motor => {
            let r = 30.0_f32.max((((last - first) / 2.0 + 10.0) / 5.0).ceil() * 5.0);
            let cy = top + 20.0 + r;
            for &x in &poles {
                c.circle(x, top, DOT, THIN, INK);
                c.line(
                    &[(x, top + DOT), (x, circle_edge(centre, cy, r, x))],
                    THIN,
                    INK,
                );
            }
            c.circle(centre, cy, r, THIN, INK);
            c.bold(centre, cy - 8.0, "M", TEXT_XL, Align::Center, INK);
            let kind = match supply {
                Supply::Ac400 => "3~",
                Supply::Ac230 => "1~",
                Supply::Dc24 => "=",
            };
            c.text(centre, cy + 16.0, kind, TEXT, Align::Center, INK);
            geo.out_y = cy + r;
            geo.pe = Some((centre, cy + r));
        }
        DeviceKind::Load => {
            let (x0, x1) = (first - 20.0, last + 20.0);
            let (y0, y1) = (top + 20.0, top + 70.0);
            for &x in &poles {
                c.circle(x, top, DOT, THIN, INK);
                c.line(&[(x, top + DOT), (x, y0)], THIN, INK);
            }
            c.rect(x0, y0, x1 - x0, y1 - y0, THIN, INK);
            c.text(
                centre,
                (y0 + y1) / 2.0,
                lang.pick("нагр.", "load"),
                TEXT,
                Align::Center,
                WEAK,
            );
            geo.out_y = y1;
            geo.pe = Some((centre, y1));
        }
        // Not drawn on the power sheet: the feed is the bus of the sheet, the rest belongs to
        // the control circuits.
        DeviceKind::Feed | DeviceKind::Terminal | DeviceKind::Sensor | DeviceKind::Button => {}
    }
    geo
}

/// The terminals of a device on the control sheet, where the wires are joined.
pub struct Field {
    /// The terminal the signal wire of the board is joined to.
    pub signal: (f32, f32),
    /// The `+24V` terminal, if the device has one.
    pub plus: Option<(f32, f32)>,
    /// The `0V` terminal, if the device has one.
    pub minus: Option<(f32, f32)>,
}

/// Room a device of the control sheet takes to each side of its terminal: the column pitch.
pub const FIELD_PITCH: f32 = 120.0;

/// Draws a coil, a sensor or a button on the control sheet, its top terminal at `(x, top)`.
/// `contacts_at` says where the contacts of a coil are drawn. `None` for kinds that have no place
/// on the control sheet.
pub fn draw_field_device(
    c: &mut Canvas,
    device: &Device,
    (x, top): (f32, f32),
    contacts_at: Option<&str>,
) -> Option<Field> {
    let name = device.designation();
    let dot = |c: &mut Canvas, x: f32, y: f32| c.circle(x, y, DOT, THIN, INK);
    match device.kind {
        DeviceKind::Contactor | DeviceKind::Relay => {
            c.rect(x - 10.0, top, 20.0, 50.0, THIN, INK);
            dot(c, x, top);
            dot(c, x, top + 50.0);
            // The designation is left of the symbol (7.1.6.1).
            c.bold(x - 18.0, top + 25.0, &name, TEXT, Align::Right, INK);
            c.text(x + 15.0, top + 8.0, "A1", TEXT, Align::Left, WEAK);
            c.text(x + 15.0, top + 42.0, "A2", TEXT, Align::Left, WEAK);
            if let Some(at) = contacts_at {
                c.text(x + 15.0, top + 25.0, at, TEXT, Align::Left, INK);
            }
            Some(Field {
                signal: (x, top),
                plus: None,
                minus: Some((x, top + 50.0)),
            })
        }
        DeviceKind::Sensor => {
            c.rect(x - 30.0, top, 60.0, 50.0, THIN, INK);
            dot(c, x, top);
            dot(c, x - 15.0, top + 50.0);
            dot(c, x + 15.0, top + 50.0);
            c.bold(x, top + 16.0, &name, TEXT, Align::Center, INK);
            c.text(x - 15.0, top + 38.0, "+", TEXT, Align::Center, WEAK);
            c.text(x + 15.0, top + 38.0, "-", TEXT, Align::Center, WEAK);
            Some(Field {
                signal: (x, top),
                plus: Some((x - 15.0, top + 50.0)),
                minus: Some((x + 15.0, top + 50.0)),
            })
        }
        DeviceKind::Button => {
            // A push button: the contact with its actuator, open; it closes to the right.
            dot(c, x, top);
            dot(c, x, top + 60.0);
            c.line(&[(x, top + DOT), (x, top + 15.0)], THIN, INK);
            c.line(&[(x, top + 15.0), (x - 10.0, top + 45.0)], THIN, INK);
            c.line(&[(x, top + 45.0), (x, top + 60.0 - DOT)], THIN, INK);
            c.dashed(&[(x - 5.0, top + 30.0), (x - 20.0, top + 30.0)], THIN, INK);
            c.line(&[(x - 20.0, top + 22.5), (x - 20.0, top + 37.5)], THIN, INK);
            c.bold(x + 15.0, top + 30.0, &name, TEXT, Align::Left, INK);
            Some(Field {
                signal: (x, top),
                plus: Some((x, top + 60.0)),
                minus: None,
            })
        }
        _ => None,
    }
}

/// Earth wire from `(x, y)` to the earth bus at `bus_y`, with the number of the wire.
pub fn draw_earth(c: &mut Canvas, (x, y): (f32, f32), bus_y: f32, number: u32) {
    c.line(&[(x, y), (x, bus_y)], THICK, EARTH);
    c.dot(x, bus_y, DOT, EARTH);
    // Left of a vertical wire, along it (7.1.3.5).
    c.vtext(
        x - 8.0,
        bus_y - 8.0,
        &number.to_string(),
        TEXT,
        Align::Left,
        EARTH,
    );
}
