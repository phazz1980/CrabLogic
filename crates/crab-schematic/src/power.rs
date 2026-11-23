//! The power sheets: feed, breakers, contactors, motors and loads on mains, laid out in columns.
//!
//! Every device feeds at most one other (`load`), so under the feed the circuits are chains. A
//! chain stands in one column, one device per row. A chain longer than the sheet holds is cut into
//! segments; the cut is marked on both sides with where the chain goes on (`next: -K1 3.B2`) and
//! where it comes from. Columns are filled sheet after sheet: first the beginnings of all chains
//! (they hang on the buses of the feed), then the continuations, so a continuation never stands
//! between a bus and the column it feeds.

use std::collections::HashMap;

use crab_core::panel_layout::label_key;
use crab_core::{bus_key, symbol_key, Device, DeviceKind, Panel, PanelLayout, SheetId};

use crate::bus::{self, Elsewhere};
use crate::canvas::{Align, Canvas, DOT, EARTH, INK, TEXT, TEXT_S, THICK, WEAK};
use crate::control::{draw_hand, Hand, Pins, Refs};
use crate::doc::{Page, Sheet};
use crate::error::Error;
use crate::lang::Lang;
use crate::nets::{mains, Nets, Wire, WireClass, EXTERNAL};
use crate::route;
use crate::scene::{Dir, End, ItemKind, Port};
use crate::symbols::{
    column_width, contacts_x, draw_device, draw_earth, draw_earth_up, Geometry, Notes, HEIGHT,
};
use crate::xref::{Xref, Xrefs};

/// Distance between the buses of the conductors below the feed.
const BUS_STEP: f32 = 20.0;
/// Rows are at most this far apart, and never closer than `MIN_PITCH`.
const MAX_PITCH: f32 = 140.0;
const MIN_PITCH: f32 = 110.0;
/// Free space kept below the lowest device for the earth bus.
const EARTH_ROOM: f32 = 70.0;
/// The most power sheets a line carried on by hand can add to the ones the devices need.
pub const MAX_EXTRA: usize = 20;
/// Length of the stubs that mark a cut of a chain.
const STUB: f32 = 40.0;
/// The most devices that stand across the lines at their entry.
const MAX_INPUT: usize = 3;
/// Distance of the entry of the lines (where their names are) from the lower edge of the frame.
const ENTRY_ROOM: f32 = 60.0;
/// Distance between devices across the entering lines, top to top.
const INPUT_PITCH: f32 = 100.0;
/// A wire between the entry and the first device across the lines, and the like.
const INPUT_GAP: f32 = 30.0;
/// Contact height of the devices across the lines (`symbols::CONTACT_H`).
const INPUT_H: f32 = 50.0;
/// The last line of a column caption stands this far above the bottom of the body (as on the
/// control sheets).
const CAPTION_DY: f32 = 30.0;
/// Pitch of the lines of a column caption.
const CAPTION_LINE_H: f32 = 14.0;
/// Longest text of a column caption on one line; a longer one is wrapped to two lines.
const CAPTION_CHARS: usize = 14;
/// The most lines a column caption takes.
const CAPTION_MAX_LINES: usize = 3;

/// Name of the bus of the conductor `k`: after the standard (7.4.5) the mains are `L1`, `L2`, `L3`,
/// `N` and the direct-current ones `L+`, `M`.
fn bus_label(supply: crab_core::Supply, k: usize) -> String {
    let name = mains(supply)[k];
    match (supply, name) {
        (crab_core::Supply::Dc24, "+24V") => "L+".to_owned(),
        (crab_core::Supply::Dc24, "0V") => "M".to_owned(),
        _ => name.to_owned(),
    }
}

/// The supply written in the short form of the standard: `1N ~ 50 Гц 230 В`.
fn supply_mark(supply: crab_core::Supply, lang: Lang) -> &'static str {
    match supply {
        crab_core::Supply::Ac230 => lang.pick("1N ~ 50 Гц 230 В", "1N ~ 50 Hz 230 V"),
        crab_core::Supply::Ac400 => lang.pick("3PEN ~ 50 Гц 400/230 В", "3PEN ~ 50 Hz 400/230 V"),
        crab_core::Supply::Dc24 => lang.pick("= 24 В", "= 24 V"),
    }
}

/// What feeds a device: the device (or the feed) and the wires, one per conductor.
struct Edge<'a> {
    parent: &'a str,
    wires: Vec<&'a Wire>,
}

/// Where a device stands: the sheet (counted from the first power sheet), the column and the row.
#[derive(Clone, Copy)]
struct Slot {
    page: usize,
    col: usize,
    row: usize,
    /// The device stands across the entering lines at the left of the first sheet, not in a column.
    input: bool,
}

/// Geometry shared by all the sheets.
struct Grid {
    page: Page,
    col_w: f32,
    first_row: f32,
    pitch: f32,
    /// Where the lines of the feed come in, at the lower edge of the first sheet.
    entry_y: f32,
}

impl Grid {
    fn left(&self, col: usize) -> f32 {
        self.page.x0() + 5.0 + col as f32 * self.col_w
    }

    fn top(&self, row: usize) -> f32 {
        self.first_row + row as f32 * self.pitch
    }

    /// Top of the device `row` of the stack across the entering lines (the first one is lowest).
    fn input_top(&self, row: usize) -> f32 {
        self.entry_y - INPUT_GAP - INPUT_H - row as f32 * INPUT_PITCH
    }

    /// Where the symbol in `slot` stands once the manual `shift` is applied; it stays inside the
    /// frame of the sheet.
    fn place(&self, slot: Slot, (dx, dy): (f32, f32)) -> (f32, f32) {
        let top = if slot.input {
            self.input_top(slot.row)
        } else {
            self.top(slot.row)
        };
        let (x, y) = (self.left(slot.col) + dx, top + dy);
        let (min_x, max_x) = (self.page.x0() + 5.0, self.page.x1() - self.col_w);
        let (min_y, max_y) = (self.page.y0() + 20.0, self.page.y1() - HEIGHT - 10.0);
        (x.clamp(min_x, max_x.max(min_x)), y.clamp(min_y, max_y))
    }

    /// Zone of the contacts of the device in `slot`.
    fn zone(&self, slot: Slot, shift: (f32, f32)) -> String {
        let (x, y) = self.place(slot, shift);
        self.page.zone(contacts_x(x), y + 50.0)
    }
}

/// The power sheets of the panel; none if the panel has no power circuits.
///
/// The first sheet is number `first_sheet`; `coils` says where the coils of the contactors and
/// relays and the auxiliary contacts are drawn (for the references under them). Returns the sheets,
/// the place of every device drawn on them (the contacts of a device), and the places of the coils
/// and auxiliary contacts put on these sheets by hand.
pub fn power_sheets(
    panel: &Panel,
    layout: &PanelLayout,
    nets: &Nets,
    lang: Lang,
    first_sheet: usize,
    coils: &Xrefs,
    page: &Page,
) -> Result<(Vec<Sheet>, Xrefs, Xrefs), Error> {
    let n = mains(panel.supply).len();
    let feed = panel.devices.iter().find(|d| d.kind == DeviceKind::Feed);
    let root = feed.map_or(EXTERNAL, |f| f.id.as_str());

    let mut edges: HashMap<&str, Edge> = HashMap::new();
    let mut earth: HashMap<&str, &Wire> = HashMap::new();
    for w in &nets.wires {
        match w.class {
            WireClass::Power => edges
                .entry(w.to.device.as_str())
                .or_insert_with(|| Edge {
                    parent: w.from.device.as_str(),
                    wires: Vec::new(),
                })
                .wires
                .push(w),
            WireClass::Protective => {
                earth.insert(w.to.device.as_str(), w);
            }
            _ => {}
        }
    }
    // Who feeds whom, in the order of the panel; the chains start at the feed.
    let mut next_of: HashMap<&str, &Device> = HashMap::new();
    let mut starts: Vec<&Device> = Vec::new();
    // Devices on a branch of another device (`from`): chains of their own, hung on that device.
    let mut branches: Vec<&Device> = Vec::new();
    let is_branch = |d: &Device| {
        panel
            .device(&d.from)
            .is_some_and(|s| s.kind != DeviceKind::Feed)
    };
    for d in &panel.devices {
        if let Some(edge) = edges.get(d.id.as_str()) {
            if is_branch(d) {
                branches.push(d);
            } else if edge.parent == root {
                starts.push(d);
            } else {
                next_of.entry(edge.parent).or_insert(d);
            }
        }
    }
    if starts.is_empty() && branches.is_empty() {
        return Ok((Vec::new(), Xrefs::new(), Xrefs::new()));
    }
    let mut chains: Vec<Vec<&Device>> = starts
        .iter()
        .map(|&first| chain_from(first, &next_of, panel.devices.len()))
        .collect();
    // With one circuit from the feed, its first protective or switching devices stand across the
    // lines at the entry (a main switch, fuses); the rest of the chain hangs on the buses.
    let mut input: Vec<&Device> = Vec::new();
    if let [only] = chains.as_mut_slice() {
        let count = only
            .iter()
            .take(MAX_INPUT)
            .take_while(|d| {
                matches!(
                    d.kind,
                    DeviceKind::Breaker | DeviceKind::Rcd | DeviceKind::Fuse | DeviceKind::Switch
                )
            })
            .count();
        input = only.drain(..count).collect();
    }
    chains.retain(|c| !c.is_empty());
    chains.extend(
        branches
            .iter()
            .map(|&first| chain_from(first, &next_of, panel.devices.len())),
    );

    // Rows: as far apart as the rows of a sheet allow; a longer chain is cut.
    let feed_h = 80.0 + n as f32 * BUS_STEP;
    let first_row = page.y0() + feed_h;
    let earth_y = page.y1() - 30.0;
    let room = earth_y - EARTH_ROOM - first_row - HEIGHT;
    let max_rows = (room / MIN_PITCH) as usize + 1;
    let longest = chains.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let rows = longest.min(max_rows);
    let pitch = if rows > 1 {
        (room / (rows - 1) as f32).min(MAX_PITCH)
    } else {
        MAX_PITCH
    };
    let aux_most = panel
        .devices
        .iter()
        .map(|d| match d.kind {
            DeviceKind::Breaker | DeviceKind::Rcd => panel.aux_of(&d.id).count(),
            DeviceKind::Motor => panel.sensors_of(&d.id).count(),
            _ => 0,
        })
        .max()
        .unwrap_or(0)
        .min(4);
    let col_w = column_width(n, aux_most);
    let grid = Grid {
        page: *page,
        col_w,
        first_row,
        pitch,
        entry_y: page.y1() - ENTRY_ROOM,
    };
    // X of the entering line of conductor `k`.
    let line_x = |k: usize| grid.left(0) + contacts_x(0.0) + k as f32 * crate::symbols::POLE;

    // Segments: the first ones of all chains, then the second ones, and so on.
    let capacity = ((page.x1() - page.x0() - 10.0) / col_w) as usize;
    let mut slot: HashMap<&str, Slot> = HashMap::new();
    for (row, d) in input.iter().enumerate() {
        slot.insert(
            &d.id,
            Slot {
                page: 0,
                col: 0,
                row,
                input: true,
            },
        );
    }
    // The first column of the first sheet is the entry of the lines.
    let first_units = capacity.saturating_sub(1).max(1);
    let mut units = 0;
    for part in 0.. {
        let mut any = false;
        for chain in &chains {
            let Some(segment) = chain.chunks(rows).nth(part) else {
                continue;
            };
            any = true;
            let (page, col) = if units < first_units {
                (0, units + 1)
            } else {
                let rest = units - first_units;
                (1 + rest / capacity, rest % capacity)
            };
            for (row, d) in segment.iter().enumerate() {
                slot.insert(
                    &d.id,
                    Slot {
                        page,
                        col,
                        row,
                        input: false,
                    },
                );
            }
            units += 1;
        }
        if !any {
            break;
        }
    }
    // A line carried on to a sheet the document does not have yet makes the sheets that are
    // missing (empty power sheets with the lines); at most `MAX_EXTRA` of them.
    let placed_sheets = slot.values().map(|s| s.page).max().unwrap_or(0) + 1;
    let wanted = layout
        .bus_to
        .values()
        .map(|to| (to + 1).saturating_sub(first_sheet))
        .max()
        .unwrap_or(0);
    let total = placed_sheets.max(wanted.min(placed_sheets + MAX_EXTRA));
    // A motor, a load or a power supply gets its earth from the terminal block above it when the
    // block stands in the same column of the same sheet: the earth wire then runs straight up.
    let earth_terminal = |d: &Device| -> Option<&str> {
        if !matches!(
            d.kind,
            DeviceKind::Motor | DeviceKind::Load | DeviceKind::Psu
        ) {
            return None;
        }
        let from = earth.get(d.id.as_str())?.from.device.as_str();
        let (a, b) = (slot.get(from)?, slot.get(d.id.as_str())?);
        let is_terminal = panel
            .device(from)
            .is_some_and(|f| f.kind == DeviceKind::Terminal);
        (is_terminal && a.page == b.page && a.col == b.col).then_some(from)
    };
    let shift_of = |id: &str| layout.shift(&symbol_key("power", id));
    let at = |id: &str| -> Option<Xref> {
        let s = *slot.get(id)?;
        Some(Xref {
            sheet: first_sheet + s.page,
            zone: grid.zone(s, shift_of(id)),
        })
    };
    let places: Xrefs = slot
        .keys()
        .filter_map(|&id| Some((id.to_owned(), at(id)?)))
        .collect();

    // Which sheets have the bus of which conductor: the ones with a drop from the feed.
    let mut has_bus = vec![vec![false; n]; total];
    for (id, s) in &slot {
        if let Some(edge) = edges.get(id) {
            if !s.input && slot.get(edge.parent).is_none_or(|p| p.input) {
                for has in has_bus[s.page].iter_mut().take(edge.wires.len()) {
                    *has = true;
                }
            }
        }
    }
    // The lines of the feed come in on the first sheet and turn into their buses there.
    if let Some(first) = has_bus.first_mut() {
        first.fill(true);
    }

    // Carried on by hand: the lines of a sheet go straight on to the sheet named. The sheets
    // between do not get them; they keep only the lines of their own drops. Ascending order lets
    // a chain (0 to 2, then 2 to 4) pass the lines along. The references follow the sheet named:
    // the end on the source refers to the target, the start on the target refers back to the
    // source, also when a sheet between has a bus of its own.
    let mut carried_to: Vec<Option<usize>> = vec![None; total];
    let mut carried_from: Vec<Vec<Option<usize>>> = vec![vec![None; n]; total];
    for index in 0..total {
        if let Some(to) = layout.bus_to(index) {
            let last = to.saturating_sub(first_sheet).min(total.saturating_sub(1));
            if last > index {
                carried_to[index] = Some(last);
                for k in 0..n {
                    if has_bus[index][k] {
                        has_bus[last][k] = true;
                        carried_from[last][k] = Some(index);
                    }
                }
            }
        }
    }

    // The devices the generator draws, on the power sheets or on the control sheets: a hand
    // placement of them is not drawn again.
    let mut generated = crate::control::generated_on_control(panel, nets);
    generated.extend(slot.keys().map(|id| (*id).to_owned()));
    let mut sheets = Vec::new();
    // The coils and auxiliary contacts put on these sheets by hand.
    let mut hand = Xrefs::new();
    for index in 0..total {
        let mut placed: Vec<(&Device, Slot)> = panel
            .devices
            .iter()
            .filter_map(|d| Some((d, *slot.get(d.id.as_str())?)))
            .filter(|(_, s)| s.page == index)
            .collect();
        placed.sort_by_key(|(_, s)| (s.col, s.row));
        let mut c = Canvas::new();
        c.use_labels(layout);
        c.set_sheet(&format!("p{index}"));
        // The line can be moved by hand, but not above the frame.
        let bus_y = |k: usize| {
            (page.y0() + 40.0 + k as f32 * BUS_STEP + layout.shift(&bus_key(index, k)).1)
                .max(page.y0() + 15.0)
        };

        // The feed: its label and the buses of the conductors.
        match feed {
            Some(f) => {
                c.bold(
                    page.x0() + 2.0,
                    page.y0() + 10.0,
                    &f.designation(),
                    TEXT,
                    Align::Left,
                    INK,
                );
                c.text(
                    page.x0() + 50.0,
                    page.y0() + 10.0,
                    &f.rating,
                    TEXT,
                    Align::Left,
                    WEAK,
                );
            }
            None => c.text(
                page.x0() + 2.0,
                page.y0() + 10.0,
                lang.pick("Питание (внешнее)", "External supply"),
                TEXT,
                Align::Left,
                WEAK,
            ),
        }

        // The kind of the supply, abbreviated as IEC 61293 does (7.1.3.5).
        c.text(
            page.x0() + 400.0,
            page.y0() + 10.0,
            supply_mark(panel.supply, lang),
            TEXT,
            Align::Left,
            INK,
        );

        // Devices.
        let mut geos: HashMap<&str, Geometry> = HashMap::new();
        for &(d, s) in &placed {
            let coil_at = coils.get(&d.id).map(ToString::to_string);
            let aux_at: Vec<(crab_core::ContactForm, Option<String>, (String, String))> = panel
                .aux_of(&d.id)
                .map(|a| {
                    (
                        a.form,
                        coils.get(&a.id).map(ToString::to_string),
                        crate::nets::aux_terminals(panel, a),
                    )
                })
                .collect();
            let sensors: Vec<(String, Option<String>)> = panel
                .sensors_of(&d.id)
                .map(|b| (b.id.clone(), coils.get(&b.id).map(ToString::to_string)))
                .collect();
            let notes = Notes {
                coil_at: coil_at.as_deref(),
                aux: &aux_at,
                sensors: &sensors,
                sheet: first_sheet + index,
                pe_top: earth_terminal(d).is_some(),
            };
            let pos = grid.place(s, shift_of(&d.id));
            let item = c.begin(ItemKind::Symbol {
                view: "power",
                device: d.id.clone(),
                origin: pos,
                ports: Vec::new(),
            });
            // Only the conductors this device gets are drawn as poles.
            let poles = edges
                .get(d.id.as_str())
                .map_or(n, |e| e.wires.len().clamp(1, n));
            let geo = draw_device(&mut c, d, pos, poles, panel.supply, lang, &notes);
            c.end();
            for (k, &x) in geo.poles.iter().enumerate() {
                if s.input {
                    // The lines go up through it: they come in at its lower end.
                    c.port(item, port(format!("in:{k}"), x, geo.out_y, Dir::Down));
                    c.port(item, port(format!("out:{k}"), x, geo.in_y, Dir::Up));
                    continue;
                }
                c.port(item, port(format!("in:{k}"), x, geo.in_y, Dir::Up));
                if geo.out_y != geo.in_y {
                    c.port(item, port(format!("out:{k}"), x, geo.out_y, Dir::Down));
                }
            }
            if let Some((x, y)) = geo.pe {
                c.port(item, port("pe".to_owned(), x, y, Dir::Down));
            }
            if let Some((x, y)) = geo.pe_top {
                let dir = if d.kind == DeviceKind::Terminal {
                    Dir::Down
                } else {
                    Dir::Up
                };
                c.port(item, port("pe".to_owned(), x, y, dir));
            }
            geos.insert(&d.id, geo);
        }

        // Wires: from the buses, between the devices of a segment, and the marks of the cuts.
        let mut bus_end = vec![f32::MIN; n];
        for &(d, s) in &placed {
            let Some(edge) = edges.get(d.id.as_str()) else {
                continue;
            };
            let child = &geos[d.id.as_str()];
            let parent = slot.get(edge.parent).copied();
            if s.input {
                // Across the entering lines: the wire comes from below (the entry or the device
                // under this one) to the lower end of the device.
                let from_y = parent.map_or(grid.entry_y, |_| geos[edge.parent].in_y);
                for (k, w) in edge.wires.iter().enumerate().take(n) {
                    let cx = child.poles[k];
                    c.begin(ItemKind::Wire {
                        key: w.key(),
                        from: End::new(edge.parent, format!("out:{k}")),
                        to: End::new(&d.id, format!("in:{k}")),
                    });
                    c.line(&[(cx, from_y), (cx, child.out_y)], THICK, INK);
                    c.wire_number(
                        &w.label_key(),
                        (cx, (from_y + child.out_y) / 2.0),
                        &w.number.to_string(),
                        INK,
                    );
                    c.end();
                }
                continue;
            }
            // A device on a branch (`from`) of a device in another column of the sheet: the branch
            // leaves the line under that device at a dot (a junction) and runs to the column.
            let branch = parent
                .filter(|p| is_branch(d) && !p.input && p.page == s.page && p.col != s.col)
                .map(|_| &geos[edge.parent]);
            let from_bus = branch.is_none() && parent.is_none_or(|p| p.input);
            let joined = parent.is_some_and(|p| !p.input && p.page == s.page && p.col == s.col);
            for (k, w) in edge.wires.iter().enumerate().take(n) {
                let cx = child.poles[k];
                c.begin(ItemKind::Wire {
                    key: w.key(),
                    from: End::new(edge.parent, format!("out:{k}")),
                    to: End::new(&d.id, format!("in:{k}")),
                });
                if let Some(up) = branch {
                    // Down from the source, along to the column, up its left side, along above
                    // it and down to the device; the dot marks where the line divides.
                    let px = up.poles[k];
                    let jy = up.out_y + 12.0 + k as f32 * 4.0;
                    let gx = grid.left(s.col) + 2.0 + k as f32 * 3.0;
                    let yb = child.in_y - 8.0 - (n - 1 - k) as f32 * 4.0;
                    c.line(
                        &[
                            (px, up.out_y),
                            (px, jy),
                            (gx, jy),
                            (gx, yb),
                            (cx, yb),
                            (cx, child.in_y),
                        ],
                        THICK,
                        INK,
                    );
                    c.dot(px, jy, DOT, INK);
                    c.wire_number(
                        &w.label_key(),
                        (gx, (jy + yb) / 2.0),
                        &w.number.to_string(),
                        INK,
                    );
                    c.end();
                    continue;
                }
                let mut label_y = None;
                if from_bus {
                    // Under the feed: a drop from the bus of the conductor.
                    let y = bus_y(k);
                    c.line(&[(cx, y), (cx, child.in_y)], THICK, INK);
                    c.dot(cx, y, DOT, INK);
                    bus_end[k] = bus_end[k].max(cx);
                } else if joined {
                    let up = &geos[edge.parent];
                    let r = route::drop(
                        (up.poles[k], up.out_y),
                        (cx, child.in_y),
                        layout.bend(&w.key()),
                        route::stagger(k, n, cx - up.poles[k]),
                    );
                    c.line(&r.points, THICK, INK);
                    label_y = Some(r.label_y);
                } else {
                    // The chain comes from another sheet or column: a stub with the number.
                    c.line(&[(cx, child.in_y - STUB), (cx, child.in_y)], THICK, INK);
                }
                // The number runs along the wire, left of it (7.1.3.5).
                let from_y = if from_bus {
                    bus_y(k)
                } else if joined {
                    geos[edge.parent].out_y
                } else {
                    child.in_y - STUB
                };
                let label = w.number.to_string();
                c.wire_number(
                    &w.label_key(),
                    (cx, label_y.unwrap_or((from_y + child.in_y) / 2.0)),
                    &label,
                    INK,
                );
                c.end();
            }
            if !joined && !from_bus && branch.is_none() {
                if let (Some(from), Some(up)) = (at(edge.parent), panel.device(edge.parent)) {
                    let text = format!(
                        "{} {} {}",
                        lang.pick("от", "from"),
                        up.designation(),
                        from.from_sheet(first_sheet + index)
                    );
                    let centre = (child.poles[0] + child.poles[n - 1]) / 2.0;
                    c.text(
                        centre,
                        child.in_y - STUB - 10.0,
                        &text,
                        TEXT,
                        Align::Center,
                        INK,
                    );
                }
            }
        }
        // The other side of a cut: a stub down from the last device of a segment.
        for &(d, s) in &placed {
            let Some(&next) = next_of.get(d.id.as_str()) else {
                continue;
            };
            let Some(&to) = slot.get(next.id.as_str()) else {
                continue;
            };
            if s.input || (to.page == s.page && to.col == s.col) {
                continue;
            }
            let geo = &geos[d.id.as_str()];
            for (k, w) in edges[next.id.as_str()].wires.iter().enumerate().take(n) {
                let x = geo.poles[k];
                c.begin(ItemKind::Wire {
                    key: w.key(),
                    from: End::new(&d.id, format!("out:{k}")),
                    to: End::new(&next.id, format!("in:{k}")),
                });
                c.line(&[(x, geo.out_y), (x, geo.out_y + STUB)], THICK, INK);
                let label = w.number.to_string();
                c.wire_number(&w.label_key(), (x, geo.out_y + STUB / 2.0), &label, INK);
                c.end();
            }
            if let Some(at_next) = at(&next.id) {
                let text = format!(
                    "{} {} {}",
                    lang.pick("далее", "next"),
                    next.designation(),
                    at_next.from_sheet(first_sheet + index)
                );
                let centre = (geo.poles[0] + geo.poles[n - 1]) / 2.0;
                c.text(
                    centre,
                    geo.out_y + STUB + 10.0,
                    &text,
                    TEXT,
                    Align::Center,
                    INK,
                );
            }
        }
        // The lines come in at the lower edge of the first sheet, rise past the devices across
        // them and turn right into their buses.
        // A line a device does not switch (the neutral past a three-pole breaker) passes it: it
        // rises from the last device that has a pole for it, or from the entry.
        let riser_from = |k: usize| {
            input
                .iter()
                .rev()
                .filter_map(|d| geos.get(d.id.as_str()))
                .find(|g| g.poles.len() > k)
                .map_or(grid.entry_y, |g| g.in_y)
        };
        for (k, end) in bus_end.iter_mut().enumerate() {
            if index == 0 {
                *end = end.max(line_x(k) + 40.0);
            } else if *end == f32::MIN && has_bus[index][k] {
                // A line carried over by hand has no drops here: a short run to the frame.
                *end = page.x0() + 140.0;
            }
        }
        if index == 0 {
            for k in 0..n {
                let name = bus_label(panel.supply, k);
                let len = name.chars().count() as f32 * TEXT * 0.7;
                c.vtext(
                    line_x(k),
                    grid.entry_y + 8.0 + len,
                    &name,
                    TEXT,
                    Align::Left,
                    INK,
                );
            }
        }
        for (k, end) in bus_end.iter().enumerate() {
            if *end > f32::MIN {
                let sheet_of = |j: usize| Elsewhere {
                    sheet: first_sheet + j,
                };
                c.begin(ItemKind::Bus { index, k });
                bus::draw(
                    &mut c,
                    page,
                    bus_y(k),
                    (
                        if index == 0 {
                            line_x(k)
                        } else {
                            page.x0() + 40.0
                        },
                        *end,
                    ),
                    &bus_label(panel.supply, k),
                    &bus::Style {
                        width: THICK,
                        rgb: INK,
                        bold: false,
                        riser: (index == 0).then(|| riser_from(k)),
                    },
                    carried_from[index][k]
                        .or_else(|| (0..index).rev().find(|&j| has_bus[j][k]))
                        .map(sheet_of),
                    carried_to[index]
                        .or_else(|| (index + 1..total).find(|&j| has_bus[j][k]))
                        .map(sheet_of),
                );
                c.end();
            }
        }

        // Protective earth.
        let drops: Vec<(&Device, (f32, f32), &Wire)> = placed
            .iter()
            .filter_map(|(d, _)| {
                let at = geos.get(d.id.as_str())?.pe?;
                Some((*d, at, *earth.get(d.id.as_str())?))
            })
            .collect();
        if !drops.is_empty() {
            // Each earth wire is short and ends with the earth sign.
            for &(d, at, wire) in &drops {
                c.begin(ItemKind::Wire {
                    key: wire.key(),
                    from: End::new(&d.id, "pe"),
                    to: End::new("PE", "earth"),
                });
                draw_earth(&mut c, at, wire.number);
                c.end();
            }
        }

        // The earth of a terminal block: up from its earth terminal to the earth sign.
        for &(d, _) in &placed {
            let (Some(wire), Some((x, y))) = (
                earth.get(d.id.as_str()),
                geos.get(d.id.as_str()).and_then(|g| g.pe_top),
            ) else {
                continue;
            };
            if d.kind != DeviceKind::Terminal {
                continue;
            }
            c.begin(ItemKind::Wire {
                key: wire.key(),
                from: End::new(&d.id, "pe"),
                to: End::new("PE", "earth"),
            });
            draw_earth_up(&mut c, (x, y), wire.number);
            c.end();
        }

        // The earth wires that run up to the terminal block above.
        for &(d, _) in &placed {
            let (Some(term), Some(wire)) = (earth_terminal(d), earth.get(d.id.as_str())) else {
                continue;
            };
            let (Some(from), Some(to)) = (
                geos.get(term).and_then(|g| g.pe_top),
                geos.get(d.id.as_str()).and_then(|g| g.pe_top),
            ) else {
                continue;
            };
            c.begin(ItemKind::Wire {
                key: wire.key(),
                from: End::new(term, "pe"),
                to: End::new(&d.id, "pe"),
            });
            let r = route::drop(
                (from.0, from.1 + DOT),
                (to.0, to.1 - DOT),
                layout.bend(&wire.key()),
                0.0,
            );
            c.line(&r.points, THICK, EARTH);
            c.wire_number(
                &wire.label_key(),
                (to.0, r.label_y),
                &wire.number.to_string(),
                EARTH,
            );
            c.end();
        }

        // The captions under the columns that end in a load.
        draw_captions(&mut c, &placed, &geos, &next_of, page);

        // The devices put on the sheet by hand, and the wires drawn by hand between them.
        let mut pins = Pins::new();
        let view = Hand {
            panel,
            layout,
            nets,
            page,
            refs: Refs {
                power: &places,
                coils,
            },
            generated: &generated,
        };
        draw_hand(
            &mut c,
            &view,
            SheetId::Power(index),
            total.saturating_sub(1),
            first_sheet + index,
            &mut pins,
            &mut hand,
        );

        let title = lang.pick("Силовая часть", "Power circuit");
        let default = if total > 1 {
            format!("{title} ({}/{total})", index + 1)
        } else {
            title.to_owned()
        };
        let id = SheetId::Power(index);
        let caption = layout.caption(id).map_or(default, str::to_owned);
        sheets.push(Sheet::new(Some(id), caption, c));
    }
    Ok((sheets, places, hand))
}

/// The captions under the columns of a sheet that end in a load (motor, load, power supply): the
/// designation of the load and its rating, centred under the column. A column without a load
/// (a branch ending in a terminal, or a chain cut to the next sheet) has none.
fn draw_captions(
    c: &mut Canvas,
    placed: &[(&Device, Slot)],
    geos: &HashMap<&str, Geometry>,
    next_of: &HashMap<&str, &Device>,
    page: &Page,
) {
    let last = page.y1() - CAPTION_DY;
    for &(d, s) in placed {
        let is_load = matches!(
            d.kind,
            DeviceKind::Motor | DeviceKind::Load | DeviceKind::Psu
        );
        // The load must end its column: the lowest device there, with nothing fed from it.
        let lowest = placed
            .iter()
            .all(|(_, o)| o.input || o.page != s.page || o.col != s.col || o.row <= s.row);
        if s.input || !is_load || !lowest || next_of.contains_key(d.id.as_str()) {
            continue;
        }
        let Some(geo) = geos.get(d.id.as_str()) else {
            continue;
        };
        let (Some(&first), Some(&end)) = (geo.poles.first(), geo.poles.last()) else {
            continue;
        };
        let centre = (first + end) / 2.0;
        let mut lines = caption_lines(&d.designation());
        let rating = d.rating.trim();
        if !rating.is_empty() {
            lines.extend(caption_lines(rating));
        }
        // At most three lines keep the caption clear of the drawing above it.
        if lines.len() > CAPTION_MAX_LINES {
            lines.truncate(CAPTION_MAX_LINES);
            if let Some(tail) = lines.last_mut() {
                tail.push('…');
            }
        }
        c.begin_label(&label_key(&d.id, "column"));
        let n = lines.len();
        for (i, line) in lines.iter().enumerate() {
            let y = last - n.saturating_sub(1).saturating_sub(i) as f32 * CAPTION_LINE_H;
            c.text(centre, y, line, TEXT_S, Align::Center, INK);
        }
        c.end_label();
    }
}

/// The lines of a caption: the text, wrapped at the last space that fits when it is longer than
/// `CAPTION_CHARS`; a second line that is still too long ends with an ellipsis.
fn caption_lines(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= CAPTION_CHARS {
        return vec![text.to_owned()];
    }
    let cut = chars
        .iter()
        .take(CAPTION_CHARS + 1)
        .rposition(|c| *c == ' ')
        .filter(|&i| i > 0)
        .unwrap_or(CAPTION_CHARS);
    let first: String = chars
        .iter()
        .take(cut)
        .collect::<String>()
        .trim_end()
        .to_owned();
    let mut second: String = chars
        .iter()
        .skip(cut)
        .collect::<String>()
        .trim_start()
        .to_owned();
    if second.chars().count() > CAPTION_CHARS {
        second = second.chars().take(CAPTION_CHARS - 1).collect::<String>() + "…";
    }
    vec![first, second]
}

/// The chain that starts with `first`: each device feeds the next one.
fn chain_from<'a>(
    first: &'a Device,
    next_of: &HashMap<&str, &'a Device>,
    limit: usize,
) -> Vec<&'a Device> {
    let mut chain = vec![first];
    while let Some(&next) = next_of.get(chain[chain.len() - 1].id.as_str()) {
        // A feeding loop would never end: stop at a device seen before.
        if chain.len() > limit {
            break;
        }
        chain.push(next);
    }
    chain
}

fn port(name: String, x: f32, y: f32, dir: Dir) -> Port {
    Port { name, x, y, dir }
}
