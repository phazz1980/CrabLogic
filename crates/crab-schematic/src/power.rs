//! The power sheets: feed, breakers, contactors, motors and loads on mains, laid out in columns.
//!
//! Every device feeds at most one other (`load`), so under the feed the circuits are chains. A
//! chain stands in one column, one device per row. A chain longer than the sheet holds is cut into
//! segments; the cut is marked on both sides with where the chain goes on (`next: -K1 3.B2`) and
//! where it comes from. Columns are filled sheet after sheet: first the beginnings of all chains
//! (they hang on the buses of the feed), then the continuations, so a continuation never stands
//! between a bus and the column it feeds.

use std::collections::HashMap;

use crab_core::{Device, DeviceKind, Panel};

use crate::canvas::{Align, Canvas, DOT, EARTH, INK, TEXT, THICK, WEAK};
use crate::doc::{Page, Sheet};
use crate::error::Error;
use crate::lang::Lang;
use crate::nets::{mains, Nets, Wire, WireClass, EXTERNAL};
use crate::symbols::{column_width, contacts_x, draw_device, draw_earth, Geometry, Notes, HEIGHT};
use crate::xref::{Xref, Xrefs};

/// Distance between the buses of the conductors below the feed.
const BUS_STEP: f32 = 20.0;
/// Rows are at most this far apart, and never closer than `MIN_PITCH`.
const MAX_PITCH: f32 = 200.0;
const MIN_PITCH: f32 = 160.0;
/// Free space kept below the lowest device for the earth bus.
const EARTH_ROOM: f32 = 70.0;
/// Length of the stubs that mark a cut of a chain.
const STUB: f32 = 40.0;

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
}

/// Geometry shared by all the sheets.
struct Grid {
    page: Page,
    col_w: f32,
    first_row: f32,
    pitch: f32,
}

impl Grid {
    fn left(&self, col: usize) -> f32 {
        self.page.x0() + 5.0 + col as f32 * self.col_w
    }

    fn top(&self, row: usize) -> f32 {
        self.first_row + row as f32 * self.pitch
    }

    /// Zone of the contacts of the device in `slot`.
    fn zone(&self, slot: Slot) -> String {
        self.page
            .zone(contacts_x(self.left(slot.col)), self.top(slot.row) + 50.0)
    }
}

/// The power sheets of the panel; none if the panel has no power circuits.
///
/// The first sheet is number `first_sheet`; `coils` says where the coils of the contactors and
/// relays are drawn (for the references under them). Returns the sheets and the place of every
/// device drawn: the middle of its contacts, as `contacts_at` of the control sheet.
pub fn power_sheets(
    panel: &Panel,
    nets: &Nets,
    lang: Lang,
    first_sheet: usize,
    coils: &Xrefs,
    page: &Page,
) -> Result<(Vec<Sheet>, Xrefs), Error> {
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
    for d in &panel.devices {
        if let Some(edge) = edges.get(d.id.as_str()) {
            if edge.parent == root {
                starts.push(d);
            } else {
                next_of.entry(edge.parent).or_insert(d);
            }
        }
    }
    if starts.is_empty() {
        return Ok((Vec::new(), Xrefs::new()));
    }
    let chains: Vec<Vec<&Device>> = starts
        .iter()
        .map(|&first| {
            let mut chain = vec![first];
            while let Some(&next) = next_of.get(chain[chain.len() - 1].id.as_str()) {
                // A feeding loop would never end: stop at a device seen before.
                if chain.len() > panel.devices.len() {
                    break;
                }
                chain.push(next);
            }
            chain
        })
        .collect();

    // Rows: as far apart as the rows of a sheet allow; a longer chain is cut.
    let feed_h = 80.0 + n as f32 * BUS_STEP;
    let first_row = page.y0() + feed_h;
    let earth_y = page.y1() - 30.0;
    let room = earth_y - EARTH_ROOM - first_row - HEIGHT;
    let max_rows = (room / MIN_PITCH) as usize + 1;
    let longest = chains.iter().map(Vec::len).max().unwrap_or(1);
    let rows = longest.min(max_rows);
    let pitch = if rows > 1 {
        (room / (rows - 1) as f32).min(MAX_PITCH)
    } else {
        MAX_PITCH
    };
    let col_w = column_width(n);
    let grid = Grid {
        page: *page,
        col_w,
        first_row,
        pitch,
    };

    // Segments: the first ones of all chains, then the second ones, and so on.
    let capacity = ((page.x1() - page.x0() - 10.0) / col_w) as usize;
    let mut slot: HashMap<&str, Slot> = HashMap::new();
    let mut units = 0;
    for part in 0.. {
        let mut any = false;
        for chain in &chains {
            let Some(segment) = chain.chunks(rows).nth(part) else {
                continue;
            };
            any = true;
            let (page, col) = (units / capacity, units % capacity);
            for (row, d) in segment.iter().enumerate() {
                slot.insert(&d.id, Slot { page, col, row });
            }
            units += 1;
        }
        if !any {
            break;
        }
    }
    let total = slot.values().map(|s| s.page).max().unwrap_or(0) + 1;
    let at = |id: &str| -> Option<Xref> {
        let s = *slot.get(id)?;
        Some(Xref {
            sheet: first_sheet + s.page,
            zone: grid.zone(s),
        })
    };
    let places: Xrefs = slot
        .keys()
        .filter_map(|&id| Some((id.to_owned(), at(id)?)))
        .collect();

    let mut sheets = Vec::new();
    for index in 0..total {
        let mut placed: Vec<(&Device, Slot)> = panel
            .devices
            .iter()
            .filter_map(|d| Some((d, *slot.get(d.id.as_str())?)))
            .filter(|(_, s)| s.page == index)
            .collect();
        placed.sort_by_key(|(_, s)| (s.col, s.row));
        let mut c = Canvas::new();
        let bus_y = |k: usize| page.y0() + 40.0 + k as f32 * BUS_STEP;

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
            let tag = Some(d.bind.split('.').next().unwrap_or_default()).filter(|t| !t.is_empty());
            let coil_at = coils.get(&d.id).map(ToString::to_string);
            let notes = Notes {
                tag,
                coil_at: coil_at.as_deref(),
            };
            let pos = (grid.left(s.col), grid.top(s.row));
            let geo = draw_device(&mut c, d, pos, n, panel.supply, lang, &notes);
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
            let joined = parent.is_some_and(|p| p.page == s.page && p.col == s.col);
            for (k, w) in edge.wires.iter().enumerate().take(n) {
                let cx = child.poles[k];
                if parent.is_none() {
                    // Under the feed: a drop from the bus of the conductor.
                    let y = bus_y(k);
                    c.line(&[(cx, y), (cx, child.in_y)], THICK, INK);
                    c.dot(cx, y, DOT, INK);
                    bus_end[k] = bus_end[k].max(cx);
                } else if joined {
                    let up = &geos[edge.parent];
                    c.line(&[(up.poles[k], up.out_y), (cx, child.in_y)], THICK, INK);
                } else {
                    // The chain comes from another sheet or column: a stub with the number.
                    c.line(&[(cx, child.in_y - STUB), (cx, child.in_y)], THICK, INK);
                }
                // The number runs along the wire, left of it (7.1.3.5).
                let from_y = if parent.is_none() {
                    bus_y(k)
                } else if joined {
                    geos[edge.parent].out_y
                } else {
                    child.in_y - STUB
                };
                let label = w.number.to_string();
                c.vtext(
                    cx - 8.0,
                    (from_y + child.in_y) / 2.0,
                    &label,
                    TEXT,
                    Align::Center,
                    INK,
                );
            }
            if !joined {
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
            if to.page == s.page && to.col == s.col {
                continue;
            }
            let geo = &geos[d.id.as_str()];
            for (k, w) in edges[next.id.as_str()].wires.iter().enumerate().take(n) {
                let x = geo.poles[k];
                c.line(&[(x, geo.out_y), (x, geo.out_y + STUB)], THICK, INK);
                let label = w.number.to_string();
                c.vtext(
                    x - 8.0,
                    geo.out_y + STUB / 2.0,
                    &label,
                    TEXT,
                    Align::Center,
                    INK,
                );
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
        for (k, end) in bus_end.iter().enumerate() {
            if *end > f32::MIN {
                let y = bus_y(k);
                c.line(&[(page.x0() + 40.0, y), (*end, y)], THICK, INK);
                c.text(
                    page.x0() + 2.0,
                    y,
                    &bus_label(panel.supply, k),
                    TEXT,
                    Align::Left,
                    INK,
                );
            }
        }

        // Protective earth.
        let drops: Vec<((f32, f32), u32)> = placed
            .iter()
            .filter_map(|(d, _)| {
                let at = geos.get(d.id.as_str())?.pe?;
                Some((at, earth.get(d.id.as_str())?.number))
            })
            .collect();
        if !drops.is_empty() {
            // The bus runs below every device of the sheet (and below the marks of the cuts), so
            // that it crosses no column.
            let lowest = geos.values().map(|g| g.out_y).fold(f32::MIN, f32::max);
            let cut_below = placed.iter().any(|(d, s)| {
                next_of
                    .get(d.id.as_str())
                    .and_then(|n| slot.get(n.id.as_str()))
                    .is_some_and(|to| to.page != s.page || to.col != s.col)
            });
            let bus = (lowest + if cut_below { 70.0 } else { 40.0 }).min(earth_y);
            let end = drops.iter().map(|((x, _), _)| *x).fold(f32::MIN, f32::max);
            for &(at, number) in &drops {
                draw_earth(&mut c, at, bus, number);
            }
            c.line(&[(page.x0() + 40.0, bus), (end, bus)], THICK, EARTH);
            c.bold(page.x0() + 2.0, bus, "PE", TEXT, Align::Left, EARTH);
        }

        let title = lang.pick("Силовая часть", "Power circuit");
        sheets.push(Sheet::new(
            if total > 1 {
                format!("{title} ({}/{total})", index + 1)
            } else {
                title.to_owned()
            },
            c,
        ));
    }
    Ok((sheets, places))
}
