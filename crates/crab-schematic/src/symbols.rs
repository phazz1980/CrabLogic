//! Symbols of the panel devices, drawn after IEC 60617 (own drawings, no outside libraries).
//!
//! Every device is drawn as many-pole: one conductor per pole, `POLE` apart, the supply side at
//! the top and the load side at the bottom. All the sizes are multiples of half a module (5 units,
//! 1.25 mm), the poles and the terminals lie on the grid of GOST IEC 61082-1 7.1.2.2.
//! Movable parts are drawn as the standard asks (7.4.4): breakers and contacts open, the contact
//! moves to the right when its coil is energised.

use crab_core::panel_layout::label_key;
use crab_core::{part_key, ContactForm, Device, DeviceKind, Supply};

use crate::canvas::{
    Align, Canvas, DOT, EARTH, INK, TEXT, TEXT_L, TEXT_M, TEXT_S, THICK, THIN, WEAK,
};
use crate::control::ContactTable;
use crate::lang::Lang;
use crate::nets::BOARD;
use crate::scene::{Dir, ItemKind, Port};

/// Distance between the conductors of a device: two modules, so that text fits between the wires.
pub const POLE: f32 = 20.0;
/// Room to the left of the first pole for the designation and the rating.
const LABEL_W: f32 = 70.0;
/// Room to the right of the last pole for a coil or a caption.
const TAIL_W: f32 = 70.0;
/// The tallest symbol; the rows of a sheet are at least this far apart.
pub const HEIGHT: f32 = 90.0;
/// Height of the contact symbol of a breaker, a contactor or a relay.
const CONTACT_H: f32 = 50.0;
/// Y of the first line of text of a device (the designation), from its top terminals.
const TEXT_Y: f32 = 12.0;
/// Pitch of the lines of a rating.
const LINE_H: f32 = 15.0;

/// Distance between the auxiliary contacts drawn right of a device.
pub const AUX_PITCH: f32 = 30.0;

/// Width of the column of a device with `n` conductors and up to `aux` auxiliary contacts.
pub fn column_width(n: usize, aux: usize) -> f32 {
    LABEL_W + n.saturating_sub(1) as f32 * POLE + TAIL_W + aux as f32 * AUX_PITCH
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
    /// The terminal of the earth at the top: the one of a terminal block, or of a device that
    /// takes its earth from a terminal block above it (`Notes::pe_top`).
    pub pe_top: Option<(f32, f32)>,
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

/// An auxiliary contact of a device as drawn on the power sheet: its form, where it is used in the
/// circuit (the control sheet), its circuit terminals (`near`, `far`) and, for a changeover, its
/// normally closed terminal.
pub type AuxNote = (
    ContactForm,
    Option<String>,
    (String, String),
    Option<String>,
);

/// Captions that depend on the rest of the document.
#[derive(Default)]
pub struct Notes<'a> {
    /// Where the coil of a contactor or relay is drawn (`/2.C3`: sheet 2, zone C3).
    pub coil_at: Option<&'a str>,
    /// The auxiliary contacts of the device, in order: the form and where the contact is used
    /// in the circuit (the control sheet).
    pub aux: &'a [AuxNote],
    /// The temperature sensors of a motor: designation and where the sensor is drawn on the
    /// control sheet.
    pub sensors: &'a [(String, Option<String>)],
    /// The number of the sheet the device is on, for the numbers of the terminals.
    pub sheet: usize,
    /// The earth of the device comes from the terminal block right above it: its earth terminal
    /// is drawn at the top, next to the poles, instead of an earth sign below.
    pub pe_top: bool,
    /// The terminals of a contact of a relay in the power chain: the supply side, the load side.
    pub terminals: Option<(String, String)>,
    /// The normally closed terminal of a changeover contact of a relay (labelled, not wired).
    pub nc: Option<String>,
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

    let text_x = first - 8.0;
    // A contact of a relay is named by its relay (`-K2`), the coil of which is drawn under it.
    let name = match device.kind {
        DeviceKind::Aux => format!("-{}", device.of),
        // The main contacts of another coil carry its designation too.
        DeviceKind::Contactor if !device.of.is_empty() => format!("-{}", device.of),
        _ => device.designation(),
    };
    // The designation is left of the symbol, the technical data under it (7.1.6.1, 7.1.2.4).
    c.begin_label(&label_key(&device.id, "name"));
    c.bold(text_x, top + TEXT_Y, &name, TEXT, Align::Right, INK);
    c.end_label();
    // Under the designation of a contactor or a relay: where its coil is drawn (`/17.2`).
    let coil_line = matches!(
        device.kind,
        DeviceKind::Contactor | DeviceKind::Relay | DeviceKind::Aux
    ) && notes.coil_at.is_some();
    if let (true, Some(at)) = (coil_line, notes.coil_at) {
        c.begin_label(&label_key(&device.id, "ref"));
        c.text(text_x, top + TEXT_Y + LINE_H, at, TEXT_M, Align::Right, INK);
        c.end_label();
    }
    let lines = rating_lines(&device.rating);
    if !lines.is_empty() {
        c.begin_label(&label_key(&device.id, "rating"));
    }
    for (i, line) in lines.iter().enumerate() {
        c.text(
            text_x,
            top + TEXT_Y + (i + 1 + usize::from(coil_line)) as f32 * LINE_H,
            line,
            TEXT_M,
            Align::Right,
            WEAK,
        );
    }
    if !lines.is_empty() {
        c.end_label();
    }

    let mut geo = Geometry {
        poles: poles.clone(),
        in_y: top,
        out_y: top,
        pe: None,
        pe_top: None,
    };
    let mid = top + CONTACT_H / 2.0;
    match device.kind {
        DeviceKind::Breaker
        | DeviceKind::Rcd
        | DeviceKind::Switch
        | DeviceKind::Contactor
        | DeviceKind::Relay => {
            geo.out_y = top + CONTACT_H;
            for &x in &poles {
                c.circle(x, top, DOT, THIN, INK);
                c.circle(x, top + CONTACT_H, DOT, THIN, INK);
                let (upper, lower) = (top + CONTACT_H * 0.3, top + CONTACT_H * 0.7);
                c.line(&[(x, top + DOT), (x, upper)], THIN, INK);
                // The blade, open: it moves to the right to close (7.4.4.4).
                c.line(&[(x, upper), (x - 5.0, lower)], THIN, INK);
                c.line(&[(x, lower), (x, top + CONTACT_H - DOT)], THIN, INK);
                match device.kind {
                    DeviceKind::Contactor | DeviceKind::Relay => {
                        c.circle(x - 2.5, mid, 2.5, THIN, INK);
                    }
                    DeviceKind::Switch => {
                        // The bar across the blade of a disconnector.
                        c.line(&[(x - 5.0, lower - 3.0), (x + 1.0, lower - 3.0)], THIN, INK);
                    }
                    _ => {
                        // The cross of the breaker.
                        c.line(&[(x - 4.0, upper - 8.0), (x + 4.0, upper)], THIN, INK);
                        c.line(&[(x - 4.0, upper), (x + 4.0, upper - 8.0)], THIN, INK);
                    }
                }
                // The numbers of the terminals: odd at the top, even at the bottom.
                let pole = ((x - first) / POLE).round() as usize;
                c.begin_label(&label_key(&device.id, &format!("pins{pole}")));
                c.text(
                    x + 8.0,
                    top + 9.0,
                    &(2 * pole + 1).to_string(),
                    TEXT_S,
                    Align::Left,
                    INK,
                );
                c.text(
                    x + 8.0,
                    top + CONTACT_H - 9.0,
                    &(2 * pole + 2).to_string(),
                    TEXT_S,
                    Align::Left,
                    INK,
                );
                c.end_label();
            }
            // Mechanical link of the poles.
            if n > 1 {
                c.dashed(&[(first - 5.0, mid), (last - 5.0, mid)], THIN, INK);
            }
            if device.kind == DeviceKind::Rcd {
                // The toroid of the residual-current device around the conductors.
                c.rect(
                    first - 10.0,
                    top + CONTACT_H * 0.3 - 4.0,
                    last - first + 20.0,
                    8.0,
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
            // The contacts of a breaker or a residual-current device stand right of it; those of a
            // contactor or a relay go under its coil, on the control sheet.
            if matches!(device.kind, DeviceKind::Breaker | DeviceKind::Rcd) {
                draw_aux(c, device.kind, (last, top), notes.aux);
            }
        }
        DeviceKind::Fuse => {
            // A fuse in every conductor: a box on the line.
            geo.out_y = top + CONTACT_H;
            let mid = top + CONTACT_H / 2.0;
            for &x in &poles {
                c.circle(x, top, DOT, THIN, INK);
                c.circle(x, top + CONTACT_H, DOT, THIN, INK);
                c.line(&[(x, top + DOT), (x, top + CONTACT_H - DOT)], THIN, INK);
                c.rect(x - 4.0, mid - 9.0, 8.0, 18.0, THIN, INK);
                let pole = ((x - first) / POLE).round() as usize;
                c.begin_label(&label_key(&device.id, &format!("pins{pole}")));
                c.text(
                    x + 8.0,
                    top + 9.0,
                    &(2 * pole + 1).to_string(),
                    TEXT_S,
                    Align::Left,
                    INK,
                );
                c.text(
                    x + 8.0,
                    top + CONTACT_H - 9.0,
                    &(2 * pole + 2).to_string(),
                    TEXT_S,
                    Align::Left,
                    INK,
                );
                c.end_label();
            }
        }
        DeviceKind::Psu => {
            let (x0, x1) = (first - 20.0, last + 20.0);
            let (y0, y1) = (top + 12.0, top + 52.0);
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
            let r = 20.0_f32.max((((last - first) / 2.0 + 5.0) / 5.0).ceil() * 5.0);
            let cy = top + 18.0 + r;
            for &x in &poles {
                c.circle(x, top, DOT, THIN, INK);
                c.line(
                    &[(x, top + DOT), (x, circle_edge(centre, cy, r, x))],
                    THIN,
                    INK,
                );
            }
            // The names of the conductors at the terminals of the motor.
            for (k, &x) in poles.iter().enumerate() {
                let name = match (supply, k) {
                    (Supply::Ac400, 0) => "U",
                    (Supply::Ac400, 1) => "V",
                    (Supply::Ac400, 2) => "W",
                    (Supply::Ac230, 0) => "L",
                    (Supply::Dc24, 0) => "+",
                    (Supply::Dc24, _) => "-",
                    _ => "N",
                };
                c.text(x + 6.0, top + 8.0, name, TEXT_M, Align::Left, WEAK);
            }
            c.circle(centre, cy, r, THIN, INK);
            c.bold(centre, cy - 6.0, "M", TEXT_L, Align::Center, INK);
            let kind = match supply {
                Supply::Ac400 => "3~",
                Supply::Ac230 => "1~",
                Supply::Dc24 => "=",
            };
            c.text(centre, cy + 11.0, kind, TEXT_S, Align::Center, INK);
            geo.out_y = cy + r;
            geo.pe = Some((centre, cy + r));
            // The thermistors of the winding: a resistor with a slanted bar, two terminals.
            for (j, (name, at)) in notes.sensors.iter().enumerate() {
                let x = last + 2.0 * POLE + 14.0 + j as f32 * AUX_PITCH;
                let (y0, y1) = (top + 14.0, top + 38.0);
                c.circle(x, top, DOT, THIN, INK);
                c.circle(x, top + 52.0, DOT, THIN, INK);
                c.line(&[(x, top + DOT), (x, y0)], THIN, INK);
                c.line(&[(x, y1), (x, top + 52.0 - DOT)], THIN, INK);
                c.rect(x - 6.0, y0, 12.0, y1 - y0, THIN, INK);
                c.line(&[(x - 10.0, y1 + 2.0), (x + 10.0, y0 - 2.0)], THIN, INK);
                c.text(x + 10.0, top + 26.0, "t°", TEXT_S, Align::Left, INK);
                c.text(
                    x,
                    top + 68.0,
                    &format!("-{name}"),
                    TEXT_S,
                    Align::Center,
                    INK,
                );
                if let Some(at) = at {
                    c.text(x, top + 80.0, at, TEXT_S, Align::Center, INK);
                }
            }
        }
        DeviceKind::Load => {
            let (x0, x1) = (first - 20.0, last + 20.0);
            let (y0, y1) = (top + 12.0, top + 42.0);
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
        DeviceKind::Terminal => {
            // A row of terminals, one per conductor and the earth last; the numbers are the sheet
            // and the place on the block, along the wire (`2.1`, `2.4PE`).
            geo.out_y = top + DOT;
            for (k, &x) in poles.iter().enumerate() {
                c.circle(x, top, DOT, THIN, INK);
                c.vtext(
                    x - 8.0,
                    top - 22.0,
                    &format!("{}.{}", notes.sheet, k + 1),
                    TEXT,
                    Align::Center,
                    INK,
                );
            }
            let x = last + POLE;
            c.circle(x, top, DOT, THIN, INK);
            geo.pe_top = Some((x, top));
            c.vtext(
                x - 8.0,
                top + 28.0,
                &format!("{}.{}PE", notes.sheet, n + 1),
                TEXT,
                Align::Center,
                EARTH,
            );
        }
        // A contact of a relay on the power sheet, when it is in a chain.
        DeviceKind::Aux => {
            if let Some((inn, out)) = &notes.terminals {
                geo.out_y = top + CONTACT_H;
                draw_relay_contact(
                    c,
                    device.form,
                    poles[0],
                    top,
                    (inn, out),
                    notes.nc.as_deref(),
                );
            }
        }
        // Not drawn on the power sheet: the feed is the bus of the sheet, the rest belongs to
        // the control circuits.
        DeviceKind::Feed | DeviceKind::Sensor | DeviceKind::Button => {}
    }
    if notes.pe_top
        && matches!(
            device.kind,
            DeviceKind::Motor | DeviceKind::Load | DeviceKind::Psu
        )
    {
        // The earth comes from above: a terminal with its name next to the poles.
        let x = last + POLE;
        c.circle(x, top, DOT, THIN, INK);
        c.text(x + 6.0, top + 12.0, "PE", TEXT, Align::Left, EARTH);
        geo.pe_top = Some((x, top));
        geo.pe = None;
    }
    geo
}

/// The earth wire of a terminal block: from the earth terminal at `(x, y)` up to the earth sign,
/// with the number of the wire along it.
pub fn draw_earth_up(c: &mut Canvas, (x, y): (f32, f32), number: u32) {
    let top = y - DOT - EARTH_DROP;
    c.line(&[(x, y - DOT), (x, top)], THICK, EARTH);
    for (i, half) in [8.0, 5.0, 2.0].into_iter().enumerate() {
        let bar = top - i as f32 * 4.0;
        c.line(&[(x - half, bar), (x + half, bar)], THICK, EARTH);
    }
    c.wire_number(
        "",
        (x, y - DOT - EARTH_DROP / 2.0),
        &number.to_string(),
        EARTH,
    );
}

/// One pole of a contact of a relay in the power chain: the blade of its form, the supply terminal
/// at the top (`inn`) and the load terminal at the bottom (`out`). A normally closed contact has the
/// hook under its blade; a changeover has the hook to its normally closed terminal `nc`, which is
/// only labelled.
fn draw_relay_contact(
    c: &mut Canvas,
    form: ContactForm,
    x: f32,
    top: f32,
    (inn, out): (&str, &str),
    nc: Option<&str>,
) {
    let bottom = top + CONTACT_H;
    let (upper, lower) = (top + CONTACT_H * 0.3, top + CONTACT_H * 0.7);
    c.circle(x, top, DOT, THIN, INK);
    c.circle(x, bottom, DOT, THIN, INK);
    c.line(&[(x, top + DOT), (x, upper)], THIN, INK);
    c.line(&[(x, lower), (x, bottom - DOT)], THIN, INK);
    // Open: the blade moves to the right to close (7.4.4.4).
    c.line(&[(x, upper), (x - 5.0, lower)], THIN, INK);
    match form {
        ContactForm::No => {}
        // Normally closed: the blade rests on the hook of the fixed contact below it.
        ContactForm::Nc => c.line(&[(x - 5.0, lower), (x, lower)], THIN, INK),
        ContactForm::Changeover => {
            c.line(&[(x - 5.0, lower), (x - 10.0, lower)], THIN, INK);
            c.line(&[(x - 10.0, lower), (x - 10.0, bottom - DOT)], THIN, INK);
            c.circle(x - 10.0, bottom, DOT, THIN, INK);
            if let Some(nc) = nc {
                c.text(x - 16.0, bottom + 4.0, nc, TEXT, Align::Right, INK);
            }
        }
    }
    c.text(x + 8.0, top + 9.0, inn, TEXT_S, Align::Left, INK);
    c.text(x + 8.0, bottom - 9.0, out, TEXT_S, Align::Left, INK);
}

/// The auxiliary contacts of a breaker or a residual-current device right of its poles, joined to it
/// by the mechanical link, with the terminals and where each one is used.
fn draw_aux(c: &mut Canvas, kind: DeviceKind, (last, top): (f32, f32), aux: &[AuxNote]) {
    if aux.is_empty() {
        return;
    }
    let mid = top + CONTACT_H / 2.0;
    // After the caption of a residual-current device.
    let first = last
        + if kind == DeviceKind::Breaker {
            30.0
        } else {
            55.0
        };
    let from = last - 5.0;
    let end = first + (aux.len() - 1) as f32 * AUX_PITCH;
    let (upper, lower) = (top + CONTACT_H * 0.3, top + CONTACT_H * 0.7);
    c.dashed(&[(from, mid), (end - 5.0, mid)], THIN, INK);
    for (j, (form, _, (near, far), nc)) in aux.iter().enumerate() {
        let x = first + j as f32 * AUX_PITCH;
        c.circle(x, top, DOT, THIN, INK);
        c.circle(x, top + CONTACT_H, DOT, THIN, INK);
        c.line(&[(x, top + DOT), (x, upper)], THIN, INK);
        c.line(&[(x, lower), (x, top + CONTACT_H - DOT)], THIN, INK);
        match form {
            // Open: the blade moves to the right to close (7.4.4.4).
            ContactForm::No => c.line(&[(x, upper), (x - 5.0, lower)], THIN, INK),
            // Normally closed: the same blade rests on the hook of the fixed contact below it.
            ContactForm::Nc => {
                c.line(&[(x, upper), (x - 5.0, lower)], THIN, INK);
                c.line(&[(x - 5.0, lower), (x, lower)], THIN, INK);
            }
            // Changeover: the open blade, and a hook on the left at its lower end down to the
            // normally closed terminal.
            ContactForm::Changeover => {
                c.line(&[(x, upper), (x - 5.0, lower)], THIN, INK);
                c.line(&[(x - 5.0, lower), (x - 10.0, lower)], THIN, INK);
                c.line(
                    &[(x - 10.0, lower), (x - 10.0, top + CONTACT_H - DOT)],
                    THIN,
                    INK,
                );
                c.circle(x - 10.0, top + CONTACT_H, DOT, THIN, INK);
                if let Some(nc) = nc {
                    c.text(x - 16.0, top + CONTACT_H + 4.0, nc, TEXT, Align::Right, INK);
                }
            }
        }
        c.text(x + 6.0, top + 10.0, near, TEXT, Align::Left, INK);
        c.text(x + 6.0, top + CONTACT_H - 10.0, far, TEXT, Align::Left, INK);
    }
    // Where each contact is used, one line per contact under the group.
    let named = aux.iter().filter(|(_, at, _, _)| at.is_some()).count();
    for (j, (_, at, (near, _), _)) in aux.iter().enumerate() {
        let Some(at) = at else { continue };
        let text = if named > 1 {
            format!("{near} {at}")
        } else {
            at.clone()
        };
        c.text(
            first - 4.0,
            top + CONTACT_H + 16.0 + j as f32 * 18.0,
            &text,
            TEXT,
            Align::Left,
            INK,
        );
    }
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

/// Pitch of the rows of the contact table under a coil.
pub(crate) const TABLE_LINE_H: f32 = 14.0;
/// Half the distance between the two terminals of a contact of the table (their dots).
const TABLE_HALF_W: f32 = 16.0;
/// Distance from a terminal dot of a contact of the table to the number of the terminal.
const TABLE_NUM_DX: f32 = 4.0;
/// The reference of a contact row (`/2.2`) is right-aligned this far left of the centre.
const TABLE_REF_DX: f32 = 40.0;
/// Height of a blade of the table: from the near terminal up to the tip.
const TABLE_BLADE_H: f32 = 7.0;
/// How far the tip of a blade stops short of the far terminal.
const TABLE_TIP_GAP: f32 = 8.0;
/// Length of the hook of a normally closed contact of the table, from the tip of the blade.
const TABLE_HOOK_W: f32 = 6.0;

/// Width of the body of a coil.
const COIL_BODY_W: f32 = 30.0;
/// Height of the body of a coil.
const COIL_BODY_H: f32 = 15.0;

/// Draws a coil, a sensor or a button on the control sheet, its top terminal at `(x, top)`.
/// `contacts` is the table of the contacts of a coil (`control::contact_table`); it is drawn under
/// the coil. Kinds without a place on the control sheet draw nothing.
pub fn draw_field_device(
    c: &mut Canvas,
    device: &Device,
    (x, top): (f32, f32),
    contacts: &ContactTable,
) -> Option<Field> {
    let name = device.designation();
    let dot = |c: &mut Canvas, x: f32, y: f32| c.circle(x, y, DOT, THIN, INK);
    match device.kind {
        DeviceKind::Contactor | DeviceKind::Relay => {
            // A wide body (about 11 x 5 mm, as in cabinet documentation) between two leads; the
            // terminals stay 50 apart so the ladder keeps its rows.
            let body_top = top + 25.0 - COIL_BODY_H / 2.0;
            let body_bottom = top + 25.0 + COIL_BODY_H / 2.0;
            c.line(&[(x, top), (x, body_top)], THIN, INK);
            c.rect(
                x - COIL_BODY_W / 2.0,
                body_top,
                COIL_BODY_W,
                COIL_BODY_H,
                THIN,
                INK,
            );
            c.line(&[(x, body_bottom), (x, top + 50.0)], THIN, INK);
            dot(c, x, top);
            dot(c, x, top + 50.0);
            // The designation is left of the symbol (7.1.6.1); the terminals right of the leads.
            c.bold(
                x - COIL_BODY_W / 2.0 - 5.0,
                top + 25.0,
                &name,
                TEXT,
                Align::Right,
                INK,
            );
            c.text(x + 4.0, top + 11.0, "A1", TEXT, Align::Left, WEAK);
            c.text(x + 4.0, top + 43.0, "A2", TEXT, Align::Left, WEAK);
            // Under the A2 terminal, right of the wire that runs down from it to the 0 V bus.
            draw_table(c, x, top + 64.0, contacts);
            Some(Field {
                signal: (x, top),
                plus: None,
                minus: Some((x, top + 50.0)),
            })
        }
        DeviceKind::Sensor => {
            // The plus comes from the top (the +24 V rail), the signal and the minus go down.
            c.rect(x - 30.0, top, 60.0, 50.0, THIN, INK);
            dot(c, x - SENSOR_SIGNAL_DX, top);
            dot(c, x - SENSOR_SIGNAL_DX, top + 50.0);
            dot(c, x + SENSOR_SIGNAL_DX, top + 50.0);
            c.bold(x, top + 16.0, &name, TEXT, Align::Center, INK);
            // The terminals: `+` at the top (the `+24V` rail), `S` (the signal) and `-` at the bottom,
            // as the panel names them (`terminal_of_port`).
            c.text(
                x - SENSOR_SIGNAL_DX - SENSOR_LABEL_DX,
                top - SENSOR_LABEL_DX,
                "+",
                TEXT,
                Align::Right,
                WEAK,
            );
            c.text(
                x - SENSOR_SIGNAL_DX,
                top + 38.0,
                "S",
                TEXT,
                Align::Center,
                WEAK,
            );
            c.text(
                x + SENSOR_SIGNAL_DX,
                top + 38.0,
                "-",
                TEXT,
                Align::Center,
                WEAK,
            );
            Some(Field {
                signal: (x - SENSOR_SIGNAL_DX, top + 50.0),
                plus: Some((x - SENSOR_SIGNAL_DX, top)),
                minus: Some((x + SENSOR_SIGNAL_DX, top + 50.0)),
            })
        }
        DeviceKind::Button => {
            // A push button: the contact with its actuator, open; it closes to the right. The
            // top terminal is fed from the +24 V rail, the bottom one goes to the input.
            dot(c, x, top);
            dot(c, x, top + 60.0);
            // The terminals: `1` at the top (the `+24V` side), `2` at the bottom (the input).
            c.text(x + 6.0, top + 12.0, "1", TEXT_S, Align::Left, WEAK);
            c.text(x + 6.0, top + 52.0, "2", TEXT_S, Align::Left, WEAK);
            c.line(&[(x, top + DOT), (x, top + 15.0)], THIN, INK);
            c.line(&[(x, top + 15.0), (x - 10.0, top + 45.0)], THIN, INK);
            c.line(&[(x, top + 45.0), (x, top + 60.0 - DOT)], THIN, INK);
            c.dashed(&[(x - 5.0, top + 30.0), (x - 20.0, top + 30.0)], THIN, INK);
            c.line(&[(x - 20.0, top + 22.5), (x - 20.0, top + 37.5)], THIN, INK);
            c.bold(x + 15.0, top + 30.0, &name, TEXT, Align::Left, INK);
            Some(Field {
                signal: (x, top + 60.0),
                plus: Some((x, top)),
                minus: None,
            })
        }
        _ => None,
    }
}

/// Height of a contact symbol on the control sheet.
pub const AUX_H: f32 = 50.0;

/// Distance of the signal and of the minus terminals of a sensor from its middle; the plus is
/// above the signal.
pub(crate) const SENSOR_SIGNAL_DX: f32 = 20.0;
/// Offset of the `+` of a sensor from its top terminal, up and to the left of it.
const SENSOR_LABEL_DX: f32 = 4.0;

/// Size of the box of a channel of the board.
pub(crate) const CHANNEL_W: f32 = 60.0;
pub(crate) const CHANNEL_H: f32 = 34.0;
/// Baselines of the two lines of a channel (the tag, the terminal), from its top edge. The terminal
/// is on the top edge (an input) or on the bottom edge (an output); each line is at least 10 from
/// the terminal, so the dot and the wire do not run over the text.
const CHANNEL_TAG_DY: f32 = 10.0;
const CHANNEL_NAME_DY: f32 = 22.0;

/// A channel box of the board: a part of the board (`A1/IO5`) that stands where its terminal is
/// used. `(x, top)` is the middle of its top edge; the terminal is on the bottom edge of an output
/// (`Dir::Down`) and on the top edge of an input (`Dir::Up`). It shows the tag and `-A1:IO5`.
/// Returns the terminal and its direction; the caller notes them as the pin of the board.
pub(crate) fn draw_channel(
    c: &mut Canvas,
    terminal: &str,
    tag: &str,
    (x, top): (f32, f32),
    dir: Dir,
) -> ((f32, f32), Dir) {
    let item = c.begin(ItemKind::Symbol {
        view: "board",
        device: part_key(BOARD, terminal),
        origin: (x, top),
        ports: Vec::new(),
    });
    c.rect(x - CHANNEL_W / 2.0, top, CHANNEL_W, CHANNEL_H, THIN, INK);
    c.text(x, top + CHANNEL_TAG_DY, tag, TEXT_S, Align::Center, INK);
    c.text(
        x,
        top + CHANNEL_NAME_DY,
        &format!("-{BOARD}:{terminal}"),
        TEXT_S,
        Align::Center,
        WEAK,
    );
    let py = if dir == Dir::Up { top } else { top + CHANNEL_H };
    c.circle(x, py, DOT, THIN, INK);
    c.port(
        item,
        Port {
            name: terminal.to_owned(),
            x,
            y: py,
            dir,
        },
    );
    c.end();
    ((x, py), dir)
}

/// The contact table under a coil, one row per contact, centred on `x` from `y` down: each contact
/// is a pair of terminal numbers with the contact between them. A normally open contact has its
/// blade rising from the near terminal and stopping short of the far one; a normally closed one
/// has the blade resting on a hook that runs to the far terminal. The dashed line through the
/// blades links the rows (the poles of the device and its auxiliary contacts). The reference of
/// a row (where the contact is drawn) stands at its left; `+N` ends the table when rows are left out.
pub(crate) fn draw_table(c: &mut Canvas, x: f32, y: f32, table: &ContactTable) {
    let rows = &table.rows;
    // The first line of each row: a changeover row takes two lines.
    let mut starts = Vec::with_capacity(rows.len());
    let mut lines = 0;
    for row in rows {
        starts.push(lines);
        lines += row.lines();
    }
    let line = |first: usize, k: f32| y + (first as f32 + k) * TABLE_LINE_H;
    if let Some(last) = rows.last() {
        // The mechanical link runs from the first row to the last line of the last row, past the
        // two lines of a changeover.
        if rows.len() >= 2 {
            let top = line(starts[0], 0.5);
            let bottom = line(starts[rows.len() - 1] + last.lines() - 1, 0.5);
            c.dashed(&[(x, top), (x, bottom)], THIN, INK);
        }
    }
    let (near_x, far_x) = (x - TABLE_HALF_W, x + TABLE_HALF_W);
    for (i, row) in rows.iter().enumerate() {
        if let Some(nc) = &row.changeover {
            // Changeover, two lines: the common terminal on the lower line, the blade rising to the
            // hook of the normally closed terminal on the upper line; the normally open terminal
            // is on the lower line, at the right.
            let (up, lo) = (line(starts[i], 0.5), line(starts[i], 1.5));
            if let Some(at) = &row.at {
                c.text(x - TABLE_REF_DX, up, at, TEXT_S, Align::Right, INK);
            }
            c.text(
                near_x - TABLE_NUM_DX,
                lo,
                &row.near,
                TEXT_S,
                Align::Right,
                INK,
            );
            c.text(far_x + TABLE_NUM_DX, up, nc, TEXT_S, Align::Left, INK);
            c.text(far_x + TABLE_NUM_DX, lo, &row.far, TEXT_S, Align::Left, INK);
            c.circle(near_x, lo, DOT, THIN, INK);
            c.circle(far_x, lo, DOT, THIN, INK);
            c.circle(far_x, up, DOT, THIN, INK);
            let hook_start = (far_x - TABLE_HOOK_W, up);
            c.line(&[(near_x, lo), hook_start], THIN, INK);
            c.line(&[hook_start, (far_x, up)], THIN, INK);
            continue;
        }
        let cy = line(starts[i], 0.5);
        if let Some(at) = &row.at {
            c.text(x - TABLE_REF_DX, cy, at, TEXT_S, Align::Right, INK);
        }
        c.text(
            near_x - TABLE_NUM_DX,
            cy,
            &row.near,
            TEXT_S,
            Align::Right,
            INK,
        );
        c.text(far_x + TABLE_NUM_DX, cy, &row.far, TEXT_S, Align::Left, INK);
        c.circle(near_x, cy, DOT, THIN, INK);
        c.circle(far_x, cy, DOT, THIN, INK);
        let tip = (far_x - TABLE_TIP_GAP, cy - TABLE_BLADE_H);
        c.line(&[(near_x, cy), tip], THIN, INK);
        // Normally closed: the blade rests on a hook that runs to the far terminal.
        if row.form == ContactForm::Nc {
            let hook = (tip.0 + TABLE_HOOK_W, tip.1);
            c.line(&[tip, hook, (far_x, cy)], THIN, INK);
        }
    }
    if table.hidden > 0 {
        c.text(
            x,
            y + (lines as f32 + 0.5) * TABLE_LINE_H,
            &format!("+{}", table.hidden),
            TEXT_S,
            Align::Center,
            INK,
        );
    }
}

/// An auxiliary contact on the control sheet, drawn upright with the designation of its device
/// left of it (`-Q2`), the terminal numbers right of the ends and `at`, where the device stands on
/// the power sheet, under the designation. The signal side is the top end. A changeover has its
/// normally closed terminal `nc` on the left stub, its number left of the terminal.
pub fn draw_aux_field(
    c: &mut Canvas,
    of: &str,
    form: ContactForm,
    (x, top): (f32, f32),
    (top_label, bottom_label): (&str, &str),
    at: Option<&str>,
    nc: Option<&str>,
) -> Field {
    c.circle(x, top, DOT, THIN, INK);
    c.circle(x, top + AUX_H, DOT, THIN, INK);
    c.line(&[(x, top + DOT), (x, top + 10.0)], THIN, INK);
    c.line(&[(x, top + 40.0), (x, top + AUX_H - DOT)], THIN, INK);
    match form {
        // Open: the blade closes to the right (7.4.4.4).
        ContactForm::No => c.line(&[(x, top + 10.0), (x - 8.0, top + 40.0)], THIN, INK),
        // Normally closed: the blade rests on the hook of the fixed contact below it.
        ContactForm::Nc => {
            c.line(&[(x, top + 10.0), (x - 8.0, top + 40.0)], THIN, INK);
            c.line(&[(x - 8.0, top + 40.0), (x, top + 40.0)], THIN, INK);
        }
        // Changeover (IEC 60617): the blade goes down and left to the hook of the normally closed
        // contact, a stub 8 left of the axis that ends in its terminal at the foot; the normally
        // open terminal is on the axis, under the gap the blade leaves.
        ContactForm::Changeover => {
            let (nc_x, nc_y) = (x - 8.0, top + AUX_H);
            c.line(&[(x, top + 10.0), (nc_x, top + 30.0)], THIN, INK);
            c.line(&[(nc_x, top + 30.0), (nc_x + 5.0, top + 30.0)], THIN, INK);
            c.line(&[(nc_x, top + 30.0), (nc_x, nc_y - DOT)], THIN, INK);
            c.circle(nc_x, nc_y, DOT, THIN, INK);
            // Left of the terminal, at the height of the `14` label on the right.
            if let Some(nc) = nc {
                c.text(nc_x - 6.0, nc_y - 8.0, nc, TEXT, Align::Right, WEAK);
            }
        }
    }
    c.bold(
        x - 14.0,
        top + 25.0,
        &format!("-{of}"),
        TEXT,
        Align::Right,
        INK,
    );
    if let Some(at) = at {
        // A changeover keeps its reference above the designation: under it, the `12` label of the
        // normally closed terminal would be in the way.
        let at_y = if form == ContactForm::Changeover {
            top + 12.0
        } else {
            top + 43.0
        };
        c.text(x - 14.0, at_y, at, TEXT, Align::Right, INK);
    }
    c.text(x + 8.0, top + 8.0, top_label, TEXT, Align::Left, WEAK);
    c.text(
        x + 8.0,
        top + AUX_H - 8.0,
        bottom_label,
        TEXT,
        Align::Left,
        WEAK,
    );
    Field {
        signal: (x, top),
        plus: Some((x, top + AUX_H)),
        minus: None,
    }
}

/// Earth wire from `(x, y)` to the earth bus at `bus_y`, with the number of the wire.
/// Length of the earth wire from the device down to the earth sign.
pub const EARTH_DROP: f32 = 20.0;

pub fn draw_earth(c: &mut Canvas, (x, y): (f32, f32), number: u32) {
    let end = y + EARTH_DROP;
    c.line(&[(x, y), (x, end)], THICK, EARTH);
    // The earth sign: three bars, each shorter than the one above (IEC 60617).
    for (i, half) in [8.0, 5.0, 2.0].into_iter().enumerate() {
        let bar = end + i as f32 * 4.0;
        c.line(&[(x - half, bar), (x + half, bar)], THICK, EARTH);
    }
    // Left of a vertical wire, along it (7.1.3.5).
    c.wire_number("", (x, y + EARTH_DROP / 2.0), &number.to_string(), EARTH);
}
