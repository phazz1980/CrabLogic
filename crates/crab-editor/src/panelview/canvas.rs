//! Sheet preview, hit-testing, and mouse interaction for the «Щит» mode.

use crab_core::{bus_key, symbol_key, DeviceKind, SheetId};
use crab_schematic::{Align, Document, Format, Item, ItemKind, Lang, Op, Sheet};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Ui, Vec2};

use super::properties::{can_have_contacts, color};
use super::tree::{apply, drop_new, sheet_name_offer, Action};
use super::{refresh, Cache, DevicePayload, Drag, LayoutEdit, PalettePayload, PanelView};
use crate::app::EditorApp;
use crate::i18n::tr;

/// Where the page is on the screen: the corner and the scale (points of the screen per unit).
pub(super) struct View {
    pub(super) origin: Pos2,
    pub(super) scale: f32,
}

impl View {
    fn at(&self, (x, y): (f32, f32)) -> Pos2 {
        self.origin + Vec2::new(x, y) * self.scale
    }
}

/// The rectangle an operation covers, in the units of the page. Text is measured roughly: the
/// width of a character is half its size.
fn op_bounds(op: &Op) -> Option<Rect> {
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| {
        Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1))
    };
    match op {
        Op::Line { points, .. } => {
            let pts: Vec<Pos2> = points.iter().map(|&(x, y)| Pos2::new(x, y)).collect();
            (!pts.is_empty()).then(|| Rect::from_points(&pts))
        }
        Op::Rect { x, y, w, h, .. } => Some(rect(*x, *y, x + w, y + h)),
        Op::Circle { x, y, r, .. } | Op::Dot { x, y, r, .. } => {
            Some(rect(x - r, y - r, x + r, y + r))
        }
        Op::Text {
            x,
            y,
            text,
            size,
            align,
            vertical,
            ..
        } => {
            let w = text.chars().count() as f32 * size * 0.5;
            let (a, b) = match align {
                Align::Left => (0.0, w),
                Align::Center => (-w / 2.0, w / 2.0),
                Align::Right => (-w, 0.0),
            };
            Some(if *vertical {
                // Turned counter-clockwise: it runs up from `y`.
                rect(x - size / 2.0, y - b, x + size / 2.0, y - a)
            } else {
                rect(x + a, y - size / 2.0, x + b, y + size / 2.0)
            })
        }
    }
}

/// The reference (`/3.C4`) written at the point `p` of the sheet: the sheet it names (`None`: the
/// one it is on) and the zone.
pub(super) fn ref_at(sheet: &Sheet, p: Pos2) -> Option<(Option<usize>, String)> {
    sheet.ops.iter().rev().find_map(|op| match op {
        Op::Text { text, .. } if op_bounds(op).is_some_and(|r| r.contains(p)) => {
            crab_schematic::find_ref(text)
        }
        _ => None,
    })
}

/// The rectangle of all the operations of an item.
fn item_bounds(sheet: &Sheet, item: &Item) -> Option<Rect> {
    item.ops
        .iter()
        .filter_map(|&i| sheet.ops.get(i).and_then(op_bounds))
        .reduce(Rect::union)
}

/// The symbol under a point of the page, the topmost one.
pub(super) fn symbol_at(sheet: &Sheet, p: Pos2) -> Option<&Item> {
    sheet.items.iter().rev().find(|item| {
        matches!(item.kind, ItemKind::Symbol { .. })
            && !is_table(item)
            && item_bounds(sheet, item).is_some_and(|r| r.expand(4.0).contains(p))
    })
}

/// Whether the item is the contact table of a coil (`K1/table`): it is moved with Shift held, like
/// a text, and does not take the clicks of the symbols.
fn is_table(item: &Item) -> bool {
    item.device()
        .is_some_and(|d| crab_core::split_part(d).1 == Some("table"))
}

/// The contact table under a point of the page.
fn table_at(sheet: &Sheet, p: Pos2) -> Option<&Item> {
    sheet.items.iter().rev().find(|item| {
        is_table(item) && item_bounds(sheet, item).is_some_and(|r| r.expand(4.0).contains(p))
    })
}

/// The numbers of the first and the last power sheet of the document (from 1).
fn power_span(document: &Document) -> (usize, usize) {
    let power: Vec<usize> = document
        .sheets
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s.id, Some(crab_core::SheetId::Power(_))))
        .map(|(i, _)| i + 1)
        .collect();
    (
        power.first().copied().unwrap_or(1),
        power.last().copied().unwrap_or(1),
    )
}

/// The end reference of a supply line under a point: the power sheet it is on and the sheet it
/// goes on to (the reference right of the line, not the one at its start).
fn bus_end_at(sheet: &Sheet, p: Pos2, page_w: f32) -> Option<(usize, usize)> {
    sheet.items.iter().find_map(|item| {
        let ItemKind::Bus { index, .. } = &item.kind else {
            return None;
        };
        item.ops.iter().find_map(|&o| {
            let op = sheet.ops.get(o)?;
            let Op::Text { text, x, .. } = op else {
                return None;
            };
            let (Some(target), _) = crab_schematic::find_ref(text)? else {
                return None;
            };
            (*x > page_w / 2.0 && op_bounds(op)?.expand(3.0).contains(p))
                .then_some((*index, target))
        })
    })
}

/// How many times the usual reach of the mouse a terminal is grabbed from and dropped on.
const GRAB: f32 = 3.0;

/// The terminal of a control sheet under a point of the page: the device, the terminal and where
/// it is.
pub(super) fn port_at(
    sheet: &Sheet,
    panel: &crab_core::Panel,
    p: Pos2,
    tolerance: f32,
) -> Option<(String, String, (f32, f32))> {
    // The nearest terminal within the reach: the reach is wide, so two may be in it.
    sheet
        .items
        .iter()
        .filter_map(|item| {
            let ItemKind::Symbol {
                view,
                device,
                ports,
                ..
            } = &item.kind
            else {
                return None;
            };
            if *view != "control" && *view != "board" {
                return None;
            }
            ports
                .iter()
                .map(|pt| ((pt.x - p.x).hypot(pt.y - p.y), view, device, pt))
                .filter(|(d, ..)| *d <= tolerance)
                .min_by(|a, b| a.0.total_cmp(&b.0))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, view, device, pt)| {
            let (owner, terminal) = port_terminal(panel, view, device, &pt.name);
            (owner, terminal, (pt.x, pt.y))
        })
}

/// The places of all the terminals a wire can be drawn from or to on the sheet.
fn port_places(sheet: &Sheet) -> Vec<(f32, f32)> {
    sheet
        .items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::Symbol { view, ports, .. } if *view == "control" || *view == "board" => {
                Some(ports.iter().map(|pt| (pt.x, pt.y)))
            }
            _ => None,
        })
        .flatten()
        .collect()
}

/// The device and the terminal a port of a symbol is wired as. A part of a device (`K1/coil`,
/// the channel `A1/IO5` of the board) is wired as the device: the ends of a wire are `K1:A1`,
/// `A1:IO5`; a port of a control symbol is named by the terminal of the device.
fn port_terminal(
    panel: &crab_core::Panel,
    view: &str,
    device: &str,
    port: &str,
) -> (String, String) {
    let owner = crab_core::split_part(device).0;
    let terminal = match (view, panel.device(owner)) {
        ("control", Some(d)) => crab_schematic::control::terminal_of_port(panel, d, port),
        _ => port.to_owned(),
    };
    (owner.to_owned(), terminal)
}

/// The end of a wire under a point of the page, the nearest one: the wire key, which end it is
/// and where. Which end a point of the wire is comes from the symbol whose port it touches (a
/// signal wire is drawn from the device to the board, though its key starts at the board); a
/// point on no port counts by the drawing order: the first point is the `from` end.
pub(super) fn wire_end_at(
    sheet: &Sheet,
    p: Pos2,
    tolerance: f32,
) -> Option<(String, crab_core::WireEnd, (f32, f32))> {
    // A wire ends a dot (2.5) away from its port: the nearest port within a little more.
    let owner_at = |q: (f32, f32)| {
        sheet
            .items
            .iter()
            .filter_map(|item| match &item.kind {
                ItemKind::Symbol { device, ports, .. } => Some((device, ports)),
                _ => None,
            })
            .flat_map(|(device, ports)| {
                ports
                    .iter()
                    .map(move |pt| ((pt.x - q.0).hypot(pt.y - q.1), device))
            })
            .filter(|(d, _)| *d <= 3.0)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, device)| crab_core::split_part(device).0)
    };
    let mut best: Option<(f32, String, crab_core::WireEnd, (f32, f32))> = None;
    for item in &sheet.items {
        let ItemKind::Wire { key, from, to } = &item.kind else {
            continue;
        };
        let lines: Vec<&Vec<(f32, f32)>> = item
            .ops
            .iter()
            .filter_map(|&o| match sheet.ops.get(o) {
                Some(Op::Line { points, .. }) => Some(points),
                _ => None,
            })
            .collect();
        let (Some(&first), Some(&last)) = (
            lines.first().and_then(|l| l.first()),
            lines.last().and_then(|l| l.last()),
        ) else {
            continue;
        };
        for (q, by_order) in [
            (first, crab_core::WireEnd::From),
            (last, crab_core::WireEnd::To),
        ] {
            let d = (q.0 - p.x).hypot(q.1 - p.y);
            if d > tolerance || best.as_ref().is_some_and(|b| b.0 <= d) {
                continue;
            }
            let end = match owner_at(q) {
                Some(owner) if owner == to.device && owner != from.device => crab_core::WireEnd::To,
                Some(owner) if owner == from.device && owner != to.device => {
                    crab_core::WireEnd::From
                }
                _ => by_order,
            };
            best = Some((d, key.clone(), end, q));
        }
    }
    best.map(|(_, key, end, q)| (key, end, q))
}

/// The terminal (`DEV:terminal`) of any symbol of the sheet under a point of the page.
fn terminal_at(sheet: &Sheet, panel: &crab_core::Panel, p: Pos2, tolerance: f32) -> Option<String> {
    sheet.items.iter().find_map(|item| {
        let ItemKind::Symbol {
            view,
            device,
            ports,
            ..
        } = &item.kind
        else {
            return None;
        };
        ports
            .iter()
            .find(|pt| (pt.x - p.x).hypot(pt.y - p.y) <= tolerance)
            .map(|pt| {
                let (owner, terminal) = port_terminal(panel, view, device, &pt.name);
                format!("{owner}:{terminal}")
            })
    })
}

/// Where the end `end` of the wire `key` can be dropped: each terminal of the sheet the panel
/// accepts it on (`DEV:terminal`), with the place to mark. Each is tried on a copy of the project.
pub(super) fn rewire_targets(
    sheet: &Sheet,
    project: &crab_core::Project,
    lib: &crab_core::Library,
    key: &str,
    end: crab_core::WireEnd,
) -> Vec<(String, (f32, f32))> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in &sheet.items {
        let ItemKind::Symbol {
            view,
            device,
            ports,
            ..
        } = &item.kind
        else {
            continue;
        };
        for pt in ports {
            let (owner, terminal) = port_terminal(&project.panel, view, device, &pt.name);
            let target = format!("{owner}:{terminal}");
            if !seen.insert(target.clone()) {
                continue;
            }
            let mut copy = project.clone();
            if crab_core::rewire(&mut copy, lib, key, end, &target).is_ok() {
                out.push((target, (pt.x, pt.y)));
            }
        }
    }
    out
}

/// Why a device (or a part of one, `K1/coil`) dragged from the tree cannot be put on a sheet: the
/// generator already draws its coil there (the device is bound to an output of the board), unless
/// the coil was put on a sheet by hand and is drawn as a symbol of its own. `None`: it can.
pub(super) fn drop_refusal(
    sheets: &[Sheet],
    coil_at: &crab_schematic::Xrefs,
    key: &str,
) -> Option<&'static str> {
    let (owner, part) = crab_core::split_part(key);
    let hand_coil = crab_core::part_key(owner, crab_core::COIL);
    let coil_by_hand = sheets.iter().any(|s| {
        s.items
            .iter()
            .any(|i| i.device() == Some(hand_coil.as_str()))
    });
    // A coil (or any other part) can always be put by hand: what is put by hand wins over the
    // generator, which then leaves it out. Only the whole device the generator draws is refused.
    if part.is_some() || !coil_at.contains_key(owner) || coil_by_hand {
        return None;
    }
    Some("Это устройство уже нарисовано на листе управления")
}

/// The wire drawn by hand under a point of the page: its key.
fn link_at(sheet: &Sheet, panel: &crab_core::Panel, p: Pos2, tolerance: f32) -> Option<String> {
    sheet.items.iter().find_map(|item| {
        let ItemKind::Wire { key, .. } = &item.kind else {
            return None;
        };
        panel.links.iter().find(|l| l.key() == *key)?;
        let near = item.ops.iter().any(|&o| {
            let Some(Op::Line { points, .. }) = sheet.ops.get(o) else {
                return false;
            };
            points.windows(2).any(|w| {
                let ((x0, y0), (x1, y1)) = (w[0], w[1]);
                p.x >= x0.min(x1) - tolerance
                    && p.x <= x0.max(x1) + tolerance
                    && p.y >= y0.min(y1) - tolerance
                    && p.y <= y0.max(y1) + tolerance
            })
        });
        near.then(|| key.clone())
    })
}

/// The wire (any, drawn by hand or worked out) under a point of the page: its key.
fn wire_any_at(sheet: &Sheet, p: Pos2, tolerance: f32) -> Option<String> {
    let dist = |a: (f32, f32), b: (f32, f32)| {
        let (ab, ap) = (
            Vec2::new(b.0 - a.0, b.1 - a.1),
            Vec2::new(p.x - a.0, p.y - a.1),
        );
        let len2 = ab.length_sq();
        let t = if len2 > 0.0 {
            (ap.dot(ab) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (ap - ab * t).length()
    };
    sheet.items.iter().find_map(|item| {
        let ItemKind::Wire { key, .. } = &item.kind else {
            return None;
        };
        item.ops
            .iter()
            .any(|&o| match sheet.ops.get(o) {
                Some(Op::Line { points, .. }) => {
                    points.windows(2).any(|w| dist(w[0], w[1]) <= tolerance)
                }
                _ => false,
            })
            .then(|| key.clone())
    })
}

/// The supply line whose horizontal part is under a point of the page: its layout key.
fn bus_at(sheet: &Sheet, p: Pos2, tolerance: f32) -> Option<String> {
    bus_hit(sheet, p, tolerance).map(|(index, k)| bus_key(index, k))
}

/// The supply line whose horizontal part is under a point of the page: the power sheet (from 0)
/// and the number of the line.
fn bus_hit(sheet: &Sheet, p: Pos2, tolerance: f32) -> Option<(usize, usize)> {
    sheet.items.iter().find_map(|item| {
        let ItemKind::Bus { index, k } = &item.kind else {
            return None;
        };
        let hit = item.ops.iter().any(|&o| {
            let Some(Op::Line { points, .. }) = sheet.ops.get(o) else {
                return false;
            };
            points.windows(2).any(|w| {
                let ((x0, y0), (x1, y1)) = (w[0], w[1]);
                y0 == y1
                    && (p.y - y0).abs() <= tolerance
                    && p.x >= x0.min(x1) - tolerance
                    && p.x <= x0.max(x1) + tolerance
            })
        });
        hit.then_some((*index, *k))
    })
}

/// The text that can be moved under a point of the page: its layout key.
fn label_at(sheet: &Sheet, p: Pos2) -> Option<&str> {
    sheet.items.iter().rev().find_map(|item| {
        let ItemKind::Label { key } = &item.kind else {
            return None;
        };
        item_bounds(sheet, item)
            .is_some_and(|r| r.expand(3.0).contains(p))
            .then_some(key.as_str())
    })
}

/// The device and the layout key of a symbol that can be moved by hand (not the board).
pub(super) fn movable(item: &Item) -> Option<(&str, String)> {
    match &item.kind {
        ItemKind::Symbol { view, device, .. } if *view != "board" => {
            Some((device, symbol_key(view, device)))
        }
        _ => None,
    }
}

/// The wire whose middle segment is under a point of the page: its key and the height of the
/// segment. Only a wire routed in three segments has one.
pub(super) fn bend_at(sheet: &Sheet, p: Pos2, tolerance: f32) -> Option<(&str, f32)> {
    sheet.items.iter().find_map(|item| {
        let ItemKind::Wire { key, .. } = &item.kind else {
            return None;
        };
        let points = item.ops.iter().find_map(|&i| match sheet.ops.get(i)? {
            Op::Line { points, .. } if points.len() == 4 => Some(points),
            _ => None,
        })?;
        let (a, b) = (points[1], points[2]);
        let (lo, hi) = (a.0.min(b.0), a.0.max(b.0));
        ((p.y - a.1).abs() <= tolerance && p.x >= lo - tolerance && p.x <= hi + tolerance)
            .then_some((key.as_str(), a.1))
    })
}

/// Draws the operations of a sheet.
pub(super) fn draw_ops(painter: &egui::Painter, view: &View, ops: &[Op]) {
    // The font of the editor is a little taller than the drawing font for the same size.
    const FONT: f32 = 0.85;
    let width = |w: f32| (w * view.scale).max(0.6);
    for op in ops {
        match op {
            Op::Line {
                points,
                width: w,
                dashed,
                rgb,
            } => {
                let pts: Vec<Pos2> = points.iter().map(|&p| view.at(p)).collect();
                let stroke = Stroke::new(width(*w), color(*rgb));
                if *dashed {
                    painter.extend(egui::Shape::dashed_line(
                        &pts,
                        stroke,
                        8.0 * view.scale,
                        5.0 * view.scale,
                    ));
                } else {
                    painter.add(egui::Shape::line(pts, stroke));
                }
            }
            Op::Rect {
                x,
                y,
                w,
                h,
                width: lw,
                rgb,
            } => {
                let rect = Rect::from_min_size(view.at((*x, *y)), Vec2::new(*w, *h) * view.scale);
                painter.rect_stroke(
                    rect,
                    0.0,
                    Stroke::new(width(*lw), color(*rgb)),
                    egui::StrokeKind::Middle,
                );
            }
            Op::Circle {
                x,
                y,
                r,
                width: lw,
                rgb,
            } => {
                painter.circle_stroke(
                    view.at((*x, *y)),
                    r * view.scale,
                    Stroke::new(width(*lw), color(*rgb)),
                );
            }
            Op::Dot { x, y, r, rgb } => {
                painter.circle_filled(view.at((*x, *y)), r * view.scale, color(*rgb));
            }
            Op::Text {
                x,
                y,
                text,
                size,
                align,
                bold,
                vertical,
                rgb,
            } => {
                let font = egui::FontId::proportional(size * view.scale * FONT);
                if font.size < 3.0 {
                    continue;
                }
                let at = view.at((*x, *y));
                let ink = color(*rgb);
                // A bold line is the same text printed twice, a hair apart.
                let copies: &[f32] = if *bold { &[0.0, 0.5] } else { &[0.0] };
                if *vertical {
                    let galley = painter.layout_no_wrap(text.clone(), font, ink);
                    let (w, h) = (galley.size().x, galley.size().y);
                    // Turned by 90° counter-clockwise: the text runs up from `pos`.
                    let along = match align {
                        Align::Left => 0.0,
                        Align::Center => w / 2.0,
                        Align::Right => w,
                    };
                    for dx in copies {
                        let pos = Pos2::new(at.x - h / 2.0 + dx, at.y + along);
                        painter.add(
                            egui::epaint::TextShape::new(pos, galley.clone(), ink)
                                .with_angle(-std::f32::consts::FRAC_PI_2),
                        );
                    }
                } else {
                    let anchor = match align {
                        Align::Left => egui::Align2::LEFT_CENTER,
                        Align::Center => egui::Align2::CENTER_CENTER,
                        Align::Right => egui::Align2::RIGHT_CENTER,
                    };
                    for dx in copies {
                        painter.text(at + Vec2::new(*dx, 0.0), anchor, text, font.clone(), ink);
                    }
                }
            }
        }
    }
}

/// The preview of the sheets (the middle of the window).
pub fn sheet(app: &mut EditorApp, ui: &mut Ui) {
    refresh(app);
    if app.doc.is_none() {
        ui.centered_and_justified(|ui| {
            ui.label(tr("Откройте проект: Файл → Открыть, или создайте новый."));
        });
        return;
    }
    toolbar(app, ui);
    ui.separator();

    // The cache is taken out while the sheet is drawn: the view state changes meanwhile.
    let Some(cache) = app.panel_view.cache.take() else {
        return;
    };
    draw_sheet(app, ui, &cache);
    app.panel_view.cache = Some(cache);
}

/// Draws the sheet of the preview and handles the mouse on it.
fn draw_sheet(app: &mut EditorApp, ui: &mut Ui, cache: &Cache) {
    let document = match &cache.document {
        Ok(document) if !document.sheets.is_empty() => document,
        Ok(_) => {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(tr(
                    "Листов пока нет. Добавьте в дереве слева ввод питания, автомат и нагрузку: схема появится здесь.",
                ));
            });
            return;
        }
        Err(message) => {
            ui.colored_label(app.theme.palette.error, message);
            return;
        }
    };
    let index = app.panel_view.sheet.min(document.sheets.len() - 1);
    let lang = cache.key.lang;
    let (page_w, page_h) = document.page_size();
    let sheet = &document.sheets[index];

    let (resp, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
    let area = resp.rect;
    let view_state = &mut app.panel_view;
    let fit = (area.width() / page_w)
        .min(area.height() / page_h)
        .max(0.01);

    // Zoom around the pointer with the wheel, move with the drag.
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let old = fit * view_state.zoom;
            let new_zoom = (view_state.zoom * (1.0 + scroll * 0.002)).clamp(1.0, 12.0);
            if let Some(pointer) = resp.hover_pos() {
                let corner = area.min
                    + (area.size() - Vec2::new(page_w, page_h) * old) / 2.0
                    + view_state.pan;
                let on_page = (pointer - corner) / old;
                let new = fit * new_zoom;
                let new_corner = area.min + (area.size() - Vec2::new(page_w, page_h) * new) / 2.0;
                view_state.pan = pointer - on_page * new - new_corner;
            }
            view_state.zoom = new_zoom;
        }
    }
    // Where the page was at the start of the frame: a press on a symbol or on the middle of a
    // wire takes the drag; anything else moves the view.
    let tolerance = 6.0 / (fit * view_state.zoom);
    // A text is moved with Shift held; without it the drag takes the symbol or the wire.
    let shift_held = ui.input(|i| i.modifiers.shift);
    // With Ctrl held the end of a wire is taken to another terminal.
    let ctrl_held = ui.input(|i| i.modifiers.command);
    if resp.drag_started() {
        let scale0 = fit * view_state.zoom;
        let origin0 =
            area.min + (area.size() - Vec2::new(page_w, page_h) * scale0) / 2.0 + view_state.pan;
        view_state.drag = resp.interact_pointer_pos().and_then(|pos| {
            let p = ((pos - origin0) / scale0).to_pos2();
            // With Shift held a text is picked before the symbol and the wire it stands on.
            if let Some(key) = label_at(sheet, p).filter(|_| shift_held) {
                let shift = app
                    .doc
                    .as_ref()
                    .map_or((0.0, 0.0), |d| d.project.panel_layout.shift(key));
                return Some(Drag::Symbol {
                    key: key.to_owned(),
                    start: (p.x, p.y),
                    shift,
                });
            }
            if let (true, Some(doc)) = (ctrl_held, app.doc.as_ref()) {
                if let Some((key, end, at)) = wire_end_at(sheet, p, tolerance * 1.5) {
                    let targets = rewire_targets(sheet, &doc.project, &app.lib, &key, end);
                    return Some(Drag::Rewire {
                        key,
                        end,
                        at,
                        targets,
                    });
                }
            }
            if let Some((device, terminal, at)) = app
                .doc
                .as_ref()
                .and_then(|d| port_at(sheet, &d.project.panel, p, tolerance * GRAB))
            {
                return Some(Drag::Link {
                    from: (device, terminal),
                    at,
                });
            }
            if let Some((key, _)) = bend_at(sheet, p, tolerance) {
                return Some(Drag::Bend {
                    key: key.to_owned(),
                });
            }
            if let Some(key) = bus_at(sheet, p, tolerance) {
                let shift = app
                    .doc
                    .as_ref()
                    .map_or((0.0, 0.0), |d| d.project.panel_layout.shift(&key));
                return Some(Drag::Symbol {
                    key,
                    start: (p.x, p.y),
                    shift,
                });
            }
            if let Some((_, key)) = table_at(sheet, p).and_then(movable).filter(|_| shift_held) {
                let shift = app
                    .doc
                    .as_ref()
                    .map_or((0.0, 0.0), |d| d.project.panel_layout.shift(&key));
                return Some(Drag::Symbol {
                    key,
                    start: (p.x, p.y),
                    shift,
                });
            }
            let (_, key) = movable(symbol_at(sheet, p)?)?;
            let shift = app
                .doc
                .as_ref()
                .map_or((0.0, 0.0), |d| d.project.panel_layout.shift(&key));
            Some(Drag::Symbol {
                key,
                start: (p.x, p.y),
                shift,
            })
        });
    }
    if resp.dragged() && view_state.drag.is_none() {
        view_state.pan += resp.drag_delta();
    }
    if let Some((x, y)) = view_state.focus.take() {
        view_state.zoom = view_state.zoom.max(2.5);
        let s = fit * view_state.zoom;
        let centred = area.min + (area.size() - Vec2::new(page_w, page_h) * s) / 2.0;
        view_state.pan = area.center() - (centred + Vec2::new(x, y) * s);
    }
    if view_state.zoom <= 1.001 {
        view_state.zoom = 1.0;
        view_state.pan = Vec2::ZERO;
    }
    let scale = fit * view_state.zoom;
    let origin =
        area.min + (area.size() - Vec2::new(page_w, page_h) * scale) / 2.0 + view_state.pan;
    let view = View { origin, scale };

    let painter = painter.with_clip_rect(area);
    painter.rect_filled(area, 0.0, ui.visuals().extreme_bg_color);
    let paper = Rect::from_min_size(origin, Vec2::new(page_w, page_h) * scale);
    painter.rect_filled(paper, 0.0, Color32::WHITE);
    draw_ops(&painter, &view, &document.frame_ops(index, lang));
    draw_ops(&painter, &view, &sheet.ops);

    // The mouse: what is under it, the drag in progress, the selection.
    let page_at = |pos: Pos2| ((pos - origin) / scale).to_pos2();
    let hover = resp.hover_pos().map(page_at);
    let mut layout_edit: Option<LayoutEdit> = None;
    let mut new_link: Option<crab_core::WireLink> = None;
    let mut rewire: Option<(String, crab_core::WireEnd, String)> = None;
    let mut remove_link: Option<String> = None;
    // The picked wire deleted (worked out wires are cut) or turned into a break / back into a line.
    let mut wire_delete: Option<String> = None;
    let mut wire_break: Option<(String, bool)> = None;
    let mut unplace: Option<String> = None;
    // A contact asked for in the menu of a device on the sheet.
    let mut add_aux: Option<String> = None;
    let mut select = None;
    if let Some(drag) = view_state.drag.clone() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let (Some(pos), true) = (resp.interact_pointer_pos(), resp.dragged()) {
            let p = page_at(pos);
            match &drag {
                Drag::Symbol { key, start, shift } => {
                    layout_edit = Some(LayoutEdit::Shift {
                        // A supply line moves up and down only.
                        dx: if key.starts_with("bus:") {
                            0.0
                        } else {
                            shift.0 + p.x - start.0
                        },
                        key: key.clone(),
                        dy: shift.1 + p.y - start.1,
                    });
                }
                Drag::Bend { key } => {
                    layout_edit = Some(LayoutEdit::Bend {
                        key: key.clone(),
                        y: p.y,
                    });
                }
                // The wire being drawn follows the pointer.
                // Every terminal it may end in is marked, the one it would end in brighter.
                Drag::Link { at, from } => {
                    let color = ui.visuals().selection.stroke.color;
                    painter.line_segment([view.at(*at), pos], egui::Stroke::new(1.5, color));
                    let target = app
                        .doc
                        .as_ref()
                        .and_then(|d| port_at(sheet, &d.project.panel, p, tolerance * GRAB))
                        .filter(|(device, terminal, _)| {
                            (device.as_str(), terminal.as_str())
                                != (from.0.as_str(), from.1.as_str())
                        });
                    for q in port_places(sheet) {
                        painter.circle_stroke(view.at(q), 4.0, egui::Stroke::new(1.0, color));
                    }
                    if let Some((_, _, q)) = target {
                        painter.circle_filled(view.at(q), 6.0, color.gamma_multiply(0.5));
                        painter.circle_stroke(view.at(q), 6.0, egui::Stroke::new(2.0, color));
                    }
                }
                // The end being moved follows the pointer; the terminals it may go to are marked,
                // the one under the pointer brighter.
                Drag::Rewire { at, targets, .. } => {
                    let color = ui.visuals().selection.stroke.color;
                    painter.line_segment([view.at(*at), pos], egui::Stroke::new(1.5, color));
                    for (_, q) in targets {
                        let near = (q.0 - p.x).hypot(q.1 - p.y) <= tolerance * 1.5;
                        let r = if near { 6.0 } else { 4.0 };
                        painter.circle_stroke(view.at(*q), r, egui::Stroke::new(1.5, color));
                    }
                }
            }
        }
        if resp.drag_stopped() {
            view_state.drag = None;
            if let Drag::Rewire { key, end, .. } = &drag {
                // Let go over a terminal: the core decides, a refusal is shown with its reason.
                let target = resp.interact_pointer_pos().map(page_at).and_then(|p| {
                    app.doc
                        .as_ref()
                        .and_then(|d| terminal_at(sheet, &d.project.panel, p, tolerance * 1.5))
                });
                if let Some(target) = target {
                    rewire = Some((key.clone(), *end, target));
                }
            } else if let Drag::Link { from, .. } = &drag {
                // Let go over another terminal: the wire is made.
                let target = resp.interact_pointer_pos().map(page_at).and_then(|p| {
                    app.doc
                        .as_ref()
                        .and_then(|d| port_at(sheet, &d.project.panel, p, tolerance * GRAB))
                });
                if let Some((device, terminal, _)) = target {
                    if (device.as_str(), terminal.as_str()) != (from.0.as_str(), from.1.as_str()) {
                        new_link = Some(crab_core::WireLink {
                            from: format!("{}:{}", from.0, from.1),
                            to: format!("{device}:{terminal}"),
                        });
                    }
                }
            } else {
                layout_edit = Some(LayoutEdit::End);
            }
        }
        ui.ctx().request_repaint();
    } else if let Some(p) = hover {
        if ctrl_held && wire_end_at(sheet, p, tolerance * 1.5).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        } else if let Some((_, _, q)) = app
            .doc
            .as_ref()
            .and_then(|d| port_at(sheet, &d.project.panel, p, tolerance * GRAB))
        {
            // The terminal a wire can be drawn from is lit.
            let color = ui.visuals().selection.stroke.color;
            painter.circle_filled(view.at(q), 6.0, color.gamma_multiply(0.5));
            painter.circle_stroke(view.at(q), 6.0, egui::Stroke::new(2.0, color));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if shift_held && label_at(sheet, p).is_some() && ref_at(sheet, p).is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else if ref_at(sheet, p).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        } else if bend_at(sheet, p, tolerance).is_some() || bus_at(sheet, p, tolerance).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        } else if symbol_at(sheet, p).is_some_and(|i| movable(i).is_some()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }
    // A right click on the end reference of a supply line edits where the line is carried on to.
    if resp.secondary_clicked() {
        let click = resp.interact_pointer_pos().map(page_at);
        view_state.link_menu = None;
        view_state.unplace_menu = None;
        if let (Some(p), Some(doc)) = (click, app.doc.as_ref()) {
            view_state.link_menu = link_at(sheet, &doc.project.panel, p, tolerance);
            if view_state.link_menu.is_none() {
                // Any device on the sheet has a menu: a hand-placed one can be taken off the
                // sheet, a switching device gets auxiliary contacts.
                view_state.unplace_menu = symbol_at(sheet, p)
                    .and_then(|i| i.device())
                    .filter(|d| {
                        doc.project.panel_layout.placed.contains_key(*d)
                            || doc
                                .project
                                .panel
                                .device(crab_core::split_part(d).0)
                                .is_some_and(|dev| can_have_contacts(dev.kind))
                    })
                    .map(str::to_owned);
            }
        }
        view_state.bus_edit = resp
            .interact_pointer_pos()
            .map(page_at)
            .and_then(|p| {
                // The reference at the end of a line that goes on, or any place of the line.
                bus_end_at(sheet, p, page_w).or_else(|| {
                    let (first_power, _) = power_span(document);
                    bus_hit(sheet, p, tolerance).map(|(index, _)| (index, index + first_power + 1))
                })
            })
            .map(|(index, shown)| {
                let manual = app
                    .doc
                    .as_ref()
                    .and_then(|d| d.project.panel_layout.bus_to(index));
                (index, manual.unwrap_or(shown))
            });
    }
    if let Some((index, mut to)) = view_state.bus_edit {
        let (first_power, last_power) = power_span(document);
        let here = index + first_power;
        resp.context_menu(|ui| {
            ui.label(tr("Продлить линии питания до листа"));
            let changed = ui
                .add(egui::DragValue::new(&mut to).range(
                    (here + 1)..=(last_power + crab_schematic::power::MAX_EXTRA).max(here + 1),
                ))
                .changed();
            if changed {
                view_state.bus_edit = Some((index, to));
                layout_edit = Some(LayoutEdit::BusTo {
                    index,
                    to: Some(to),
                });
            }
            if ui.button(tr("Автоматически")).clicked() {
                layout_edit = Some(LayoutEdit::BusTo { index, to: None });
                view_state.bus_edit = None;
                ui.close();
            }
        });
    }
    if view_state.bus_edit.is_none() {
        if let Some(key) = view_state.link_menu.clone() {
            resp.context_menu(|ui| {
                if ui.button(tr("Удалить соединение")).clicked() {
                    remove_link = Some(key.clone());
                    view_state.link_menu = None;
                    ui.close();
                }
            });
        } else if let Some(id) = view_state.unplace_menu.clone() {
            let (placed, of) = app.doc.as_ref().map_or((false, None), |doc| {
                let owner = crab_core::split_part(&id).0;
                (
                    doc.project.panel_layout.placed.contains_key(&id),
                    doc.project
                        .panel
                        .device(owner)
                        .filter(|d| can_have_contacts(d.kind))
                        .map(|d| d.id.clone()),
                )
            });
            resp.context_menu(|ui| {
                if let Some(of) = &of {
                    if ui.button(tr("Добавить доп. контакт")).clicked() {
                        add_aux = Some(of.clone());
                        view_state.unplace_menu = None;
                        ui.close();
                    }
                }
                if placed && ui.button(tr("Убрать с листа")).clicked() {
                    unplace = Some(id.clone());
                    view_state.unplace_menu = None;
                    ui.close();
                }
            });
        }
    }
    // A device dragged from the tree and dropped here is put on the sheet it is dropped on.
    let mut put: Option<(String, SheetId, f32, f32, Option<usize>)> = None;
    // `dnd_release_payload` takes the payload before it looks at its type: each kind is taken
    // only when it is the one being dragged, or the other kind would be dropped on the floor.
    let device_drop = resp
        .dnd_hover_payload::<DevicePayload>()
        .and_then(|_| resp.dnd_release_payload::<DevicePayload>());
    let palette_drop = resp
        .dnd_hover_payload::<PalettePayload>()
        .and_then(|_| resp.dnd_release_payload::<PalettePayload>());
    if let Some(payload) = device_drop {
        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
            let at = page_at(pos);
            let id = payload.0.clone();
            if let Some(why) = drop_refusal(&document.sheets, &document.coil_at, &id) {
                view_state.problem = Some(tr(why).to_owned());
            } else if let Some(target) = sheet.id {
                put = Some((id, target, at.x, at.y, Some(index + 1)));
            } else {
                // Dropped on a title or a table sheet: the device goes on the first control sheet,
                // which is shown, so that it can be put in place and wired.
                let number = document
                    .sheets
                    .iter()
                    .position(|s| s.id == Some(SheetId::Control(0)))
                    .map(|i| i + 1);
                put = Some((id, SheetId::Control(0), 80.0, 400.0, number));
                view_state.goto = Some(SheetId::Control(0));
            }
        }
    }
    // An entry of the palette dropped here: a new device is made where it is dropped.
    let mut made: Option<(PalettePayload, SheetId, Option<usize>, f32, f32)> = None;
    if let Some(payload) = palette_drop {
        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
            let at = page_at(pos);
            match sheet.id {
                Some(target) => made = Some((*payload, target, Some(index + 1), at.x, at.y)),
                None => {
                    // On a title or a table sheet: the first control sheet, which is shown. Before
                    // there is one its number is not known: the coil is named like any device.
                    let number = document
                        .sheets
                        .iter()
                        .position(|s| s.id == Some(SheetId::Control(0)))
                        .map(|i| i + 1);
                    made = Some((*payload, SheetId::Control(0), number, 80.0, 400.0));
                    view_state.goto = Some(SheetId::Control(0));
                }
            }
        }
    }
    // A click on a reference goes to the sheet and the zone it names.
    let mut jump: Option<(usize, String)> = None;
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos().map(page_at) {
            if let Some((target, zone)) = ref_at(sheet, p) {
                jump = Some((target.map_or(index, |n| n.saturating_sub(1)), zone));
            } else if let Some(key) = label_at(sheet, p).filter(|_| shift_held) {
                // A text is picked (with Shift held): its properties (turn, size) are shown at the right.
                let size = sheet
                    .items
                    .iter()
                    .find(|i| matches!(&i.kind, ItemKind::Label { key: k } if k == key))
                    .and_then(|i| i.ops.first())
                    .and_then(|&o| match sheet.ops.get(o)? {
                        Op::Text { size, .. } => Some(*size),
                        _ => None,
                    })
                    .unwrap_or(17.0);
                view_state.label = Some((key.to_owned(), size));
                view_state.selected = None;
            } else {
                // A part (`K1/coil`) selects its device.
                select = symbol_at(sheet, p)
                    .and_then(|i| i.device())
                    .map(|d| crab_core::split_part(d).0.to_owned());
                // Else a click on a wire picks it.
                view_state.wire = if select.is_none() {
                    wire_any_at(sheet, p, tolerance)
                } else {
                    None
                };
            }
        }
    }
    if resp.double_clicked() {
        if let Some((key, _)) = resp
            .interact_pointer_pos()
            .and_then(|pos| bend_at(sheet, page_at(pos), tolerance))
        {
            layout_edit = Some(LayoutEdit::ResetBend {
                key: key.to_owned(),
            });
        }
    }
    // The symbols of the selected device are framed.
    let accent = app.theme.palette.accent;
    // The picked wire is drawn thick, with the buttons «Удалить» and «Превратить в ссылку».
    if let Some(key) = view_state.wire.clone() {
        let mut here = false;
        for item in &sheet.items {
            if matches!(&item.kind, ItemKind::Wire { key: k, .. } if *k == key) {
                for &o in &item.ops {
                    if let Some(Op::Line { points, .. }) = sheet.ops.get(o) {
                        here = true;
                        let pts: Vec<Pos2> = points.iter().map(|&p| view.at(p)).collect();
                        painter.add(egui::Shape::line(pts, Stroke::new(3.0, accent)));
                    }
                }
            }
        }
        if here {
            let broken = app
                .doc
                .as_ref()
                .is_some_and(|d| d.project.panel_layout.is_broken(&key));
            egui::Area::new(ui.make_persistent_id("panel_wire_bar"))
                .fixed_pos(area.left_top() + Vec2::new(8.0, 8.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(format!("{} {key}", tr("Связь")));
                            if ui.button(tr("Удалить")).clicked() {
                                wire_delete = Some(key.clone());
                            }
                            let title = if broken {
                                tr("Вернуть линией")
                            } else {
                                tr("Превратить в ссылку")
                            };
                            if ui.button(title).clicked() {
                                wire_break = Some((key.clone(), !broken));
                            }
                        });
                    });
                });
        }
        if ui.input(|i| i.key_pressed(egui::Key::Delete)) && here {
            wire_delete = Some(key);
        }
    }
    if let Some(chosen) = &view_state.selected {
        for item in &sheet.items {
            if item.device().map(|d| crab_core::split_part(d).0) == Some(chosen.as_str()) {
                if let Some(r) = item_bounds(sheet, item) {
                    let r = Rect::from_min_max(
                        view.at((r.min.x, r.min.y)),
                        view.at((r.max.x, r.max.y)),
                    )
                    .expand(3.0);
                    painter.rect_stroke(
                        r,
                        2.0,
                        Stroke::new(1.5, accent),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        }
    }
    if let Some((on, zone)) = &view_state.highlight {
        if *on == index {
            if let Some((x, y, w, h)) = document.zone_rect(zone) {
                let rect = Rect::from_min_size(view.at((x, y)), Vec2::new(w, h) * scale);
                let accent = app.theme.palette.accent;
                painter.rect_filled(rect, 0.0, accent.gamma_multiply(0.18));
                painter.rect_stroke(
                    rect,
                    0.0,
                    Stroke::new(2.0, accent),
                    egui::StrokeKind::Middle,
                );
            }
        }
    }
    if let Some((sheet_index, zone)) = jump {
        if sheet_index < document.sheets.len() {
            view_state.focus = document
                .zone_rect(&zone)
                .map(|(x, y, w, h)| (x + w / 2.0, y + h / 2.0));
            view_state.sheet = sheet_index;
            view_state.highlight = Some((sheet_index, zone));
        }
    }
    let reset_bend = matches!(layout_edit, Some(LayoutEdit::ResetBend { .. }));
    if resp.double_clicked() && !reset_bend {
        view_state.zoom = 1.0;
        view_state.pan = Vec2::ZERO;
        view_state.highlight = None;
    }
    if let Some(id) = select {
        if app
            .doc
            .as_ref()
            .is_some_and(|d| d.project.panel.device(&id).is_some())
        {
            view_state.selected = Some(id);
            view_state.coil = false;
            view_state.label = None;
        }
    }
    if let (Some(edit), Some(doc)) = (layout_edit, app.doc.as_mut()) {
        apply_layout_edit(doc, edit);
    }
    if let (Some((key, end, target)), Some(doc)) = (rewire, app.doc.as_mut()) {
        let lib = &app.lib;
        match doc.edit_panel_project(None, |p| crab_core::rewire(p, lib, &key, end, &target)) {
            Ok(()) => app.panel_view.problem = None,
            Err(message) => {
                // The device of the moved end is selected so the reason shows in its properties.
                let moved = key.split_once('>').map(|(a, b)| match end {
                    crab_core::WireEnd::From => a,
                    crab_core::WireEnd::To => b,
                });
                if let Some(device) = moved.map(crab_core::link_device) {
                    app.panel_view.selected = Some(device.to_owned());
                    app.panel_view.coil = false;
                }
                app.panel_view.problem = Some(message);
            }
        }
    }
    if let Some(doc) = app.doc.as_mut() {
        if let Some(link) = new_link {
            let _ = doc.edit_panel::<(), ()>(None, |p| {
                if !p.links.contains(&link) {
                    p.links.push(link);
                }
                Ok(())
            });
        }
        if let Some(key) = remove_link {
            let _ = doc.edit_panel::<(), ()>(None, |p| {
                p.links.retain(|l| l.key() != key);
                Ok(())
            });
        }
        if let Some(key) = wire_delete {
            let _ = doc.edit_panel::<(), ()>(None, |p| {
                if p.links.iter().any(|l| l.key() == key) {
                    p.links.retain(|l| l.key() != key);
                } else if !p.cut.contains(&key) {
                    p.cut.push(key.clone());
                }
                Ok(())
            });
            doc.edit_panel_layout(None, |l| l.set_broken(&key, false));
            app.panel_view.wire = None;
        }
        if let Some((key, broken)) = wire_break {
            doc.edit_panel_layout(None, |l| l.set_broken(&key, broken));
        }
        if let Some(id) = unplace {
            doc.edit_panel_layout(None, |l| l.unplace(&id));
        }
        if let Some((payload, on, number, x, y)) = made {
            let id = doc
                .edit_panel_project::<_, ()>(None, |p| Ok(drop_new(p, payload, on, number, (x, y))))
                .unwrap_or_default();
            app.panel_view.selected = Some(id);
            app.panel_view.coil = payload.part == crab_schematic::PalettePart::Coil;
            app.panel_view.problem = None;
        }
        if let Some((id, target, x, y, number)) = put {
            doc.edit_panel_layout(None, |l| l.place(&id, target, x, y));
            // A coil put on another sheet is offered its designation by that sheet.
            let (owner, part) = crab_core::split_part(&id);
            if part == Some(crab_core::COIL) {
                let kind = doc.project.panel.device(owner).map(|d| d.kind);
                app.panel_view.rename_offer = match (kind, number) {
                    (Some(kind @ (DeviceKind::Contactor | DeviceKind::Relay)), Some(number)) => {
                        sheet_name_offer(owner, kind, &doc.project.panel, number)
                            .map(|new| (owner.to_owned(), new, doc.panel_revision))
                    }
                    _ => None,
                };
            }
            app.panel_view.selected = Some(id);
            app.panel_view.coil = false;
        }
    }
    if let Some(of) = add_aux {
        apply(app, Some(Action::AddAux(of)));
    }
}

/// Puts a change of the layout made by the mouse into the document.
fn apply_layout_edit(doc: &mut crate::doc::Doc, edit: LayoutEdit) {
    match edit {
        LayoutEdit::Shift { key, dx, dy } => {
            doc.edit_panel_layout(Some(format!("shift:{key}")), |l| l.set_shift(&key, dx, dy));
        }
        LayoutEdit::Bend { key, y } => {
            doc.edit_panel_layout(Some(format!("bend:{key}")), |l| l.set_bend(&key, Some(y)));
        }
        LayoutEdit::ResetBend { key } => {
            doc.edit_panel_layout(None, |l| l.set_bend(&key, None));
        }
        LayoutEdit::BusTo { index, to } => {
            doc.edit_panel_layout(Some(format!("busto:{index}")), |l| l.set_bus_to(index, to));
        }
        LayoutEdit::End => doc.end_edit(),
    }
}

/// Makes the PDF of the panel next to the project file, `<name>_panel.pdf`, in a thread.
fn start_pdf(
    project: crab_core::Project,
    lib: crab_core::Library,
    lang: Lang,
    format: Format,
) -> std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let dir = project
                .path
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_default();
            let out = dir.join(format!("{}_panel.pdf", project.display_name()));
            let pdf = crab_schematic::render_pdf_in(&project, &lib, lang, format)
                .map_err(|e| e.to_string())?;
            std::fs::write(&out, pdf).map_err(|e| e.to_string())?;
            Ok(out)
        })();
        let _ = tx.send(result);
    });
    rx
}

/// The buttons of the shown sheet: its caption, a new free sheet, removing a free sheet.
fn sheet_controls(
    view: &mut PanelView,
    doc: &mut Option<crate::doc::Doc>,
    ui: &mut Ui,
    shown: Option<SheetId>,
    title: Option<&String>,
    free_count: usize,
) {
    let Some(doc) = doc else {
        return;
    };
    // A coil moved to another sheet: the offer to give it that sheet's designation.
    if let Some((old, new, revision)) = view.rename_offer.clone() {
        // Any later change of the panel (an undo, another edit, another project) takes it back:
        // the coil may no longer stand on that sheet, or the name may be taken.
        if doc.panel_revision != revision || doc.project.panel.device(&old).is_none() {
            view.rename_offer = None;
        } else {
            let sheet = new
                .get(1..)
                .and_then(|s| s.split_once('.'))
                .map_or_else(String::new, |(n, _)| n.to_owned());
            ui.horizontal(|ui| {
                ui.label(crate::i18n::fmt(
                    tr("Катушка перенесена на лист {0}. Переименовать -{1} в -{2}?"),
                    &[&sheet, &old, &new],
                ));
                if ui.button(tr("Переименовать")).clicked() {
                    match doc.edit_panel_project(None, |p| crab_core::rename_device(p, &old, &new))
                    {
                        Ok(()) => {
                            view.selected = Some(new.clone());
                            view.problem = None;
                        }
                        Err(message) => view.problem = Some(message),
                    }
                    view.rename_offer = None;
                }
                if ui.button(tr("Оставить")).clicked() {
                    view.rename_offer = None;
                }
            });
        }
    }
    if let Some(id) = shown {
        let editing = view.caption_edit.as_ref().is_some_and(|(e, _)| *e == id);
        if !editing {
            let text = doc
                .project
                .panel_layout
                .caption(id)
                .unwrap_or("")
                .to_owned();
            view.caption_edit = Some((id, text));
        }
        if let Some((_, text)) = &mut view.caption_edit {
            let hint = title.map_or("", |t| t.as_str());
            let response = ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .desired_width(160.0),
            );
            response.clone().on_hover_text(tr(
                "Подпись листа: идёт в основную надпись и в содержание; пусто — подпись по умолчанию",
            ));
            if response.lost_focus() {
                let text = text.clone();
                if doc.project.panel_layout.caption(id).unwrap_or("") != text.trim() {
                    doc.edit_panel_layout(None, |l| l.set_caption(id, &text));
                }
            }
        }
    }
    if ui
        .button("+")
        .on_hover_text(tr("Добавить свободный лист"))
        .clicked()
    {
        let id = SheetId::Free(free_count);
        let caption = format!("{} {}", tr("Схема"), free_count + 1);
        doc.edit_panel_layout(None, |l| l.set_caption(id, &caption));
        view.goto = Some(id);
        view.caption_edit = None;
    }
    if let Some(SheetId::Free(k)) = shown {
        if ui
            .button(tr("Удалить лист"))
            .on_hover_text(tr(
                "Удалить этот свободный лист; устройства с него вернутся в дерево, следующие листы сдвинутся",
            ))
            .clicked()
        {
            doc.edit_panel_layout(None, |l| l.remove_free_sheet(k));
            view.caption_edit = None;
        }
    }
}

/// Above the sheet: the sheets, the language and the format.
fn toolbar(app: &mut EditorApp, ui: &mut Ui) {
    let Some(cache) = &app.panel_view.cache else {
        return;
    };
    let titles: Vec<String> = cache.document.as_ref().map_or_else(
        |_| Vec::new(),
        |d| d.sheets.iter().map(|s| s.title.clone()).collect(),
    );
    let ids: Vec<Option<SheetId>> = cache
        .document
        .as_ref()
        .map_or_else(|_| Vec::new(), |d| d.sheets.iter().map(|s| s.id).collect());
    let free_count = ids
        .iter()
        .filter(|id| matches!(id, Some(SheetId::Free(_))))
        .count();
    let errors = cache.report.errors().count();
    let view = &mut app.panel_view;
    ui.horizontal(|ui| {
        let count = titles.len();
        if ui
            .add_enabled(view.sheet > 0, egui::Button::new("◀"))
            .clicked()
        {
            view.sheet -= 1;
            view.highlight = None;
        }
        let shown = titles.get(view.sheet).map_or_else(String::new, |t| {
            format!("{} / {count}  {t}", view.sheet + 1)
        });
        egui::ComboBox::from_id_salt("panel_sheet")
            .selected_text(shown)
            .width(260.0)
            .show_ui(ui, |ui| {
                for (i, title) in titles.iter().enumerate() {
                    if ui
                        .selectable_value(&mut view.sheet, i, format!("{} {title}", i + 1))
                        .clicked()
                    {
                        view.highlight = None;
                    }
                }
            });
        if ui
            .add_enabled(view.sheet + 1 < count, egui::Button::new("▶"))
            .clicked()
        {
            view.sheet += 1;
            view.highlight = None;
        }
        sheet_controls(view, &mut app.doc, ui, ids.get(view.sheet).copied().flatten(), titles.get(view.sheet), free_count);
        ui.separator();
        let mut lang = view.language();
        ui.selectable_value(&mut lang, Lang::Ru, "RU");
        ui.selectable_value(&mut lang, Lang::En, "EN");
        if lang != view.language() {
            view.lang = Some(lang);
        }
        ui.separator();
        ui.selectable_value(&mut view.format, Format::A3, "A3");
        ui.selectable_value(&mut view.format, Format::A4, "A4");
        ui.separator();
        if let Some(doc) = &mut app.doc {
            let mine = view
                .selected
                .as_ref()
                .map(|id| (symbol_key("power", id), symbol_key("control", id)));
            let has_mine = mine.as_ref().is_some_and(|(a, b)| {
                doc.project.panel_layout.symbols.contains_key(a)
                    || doc.project.panel_layout.symbols.contains_key(b)
            });
            if ui
                .add_enabled(has_mine, egui::Button::new(tr("Сбросить положение")))
                .on_hover_text(tr("Вернуть выбранному устройству автоматическое положение"))
                .clicked()
            {
                if let Some((a, b)) = mine {
                    doc.edit_panel_layout(None, |l| {
                        l.clear_symbol(&a);
                        l.clear_symbol(&b);
                    });
                }
            }
            let layout = &doc.project.panel_layout;
            let any = !layout.symbols.is_empty()
                || !layout.wires.is_empty()
                || !layout.bus_to.is_empty()
                || !layout.placed.is_empty();
            if view.confirm_reset {
                ui.label(tr("Убрать всё, что расставлено вручную?"));
                if ui.button(tr("Да, разложить")).clicked() {
                    doc.edit_panel_layout(None, |l| {
                        l.clear_all();
                        l.placed.clear();
                    });
                    view.confirm_reset = false;
                }
                if ui.button(tr("Отмена")).clicked() {
                    view.confirm_reset = false;
                }
            } else if ui
                .add_enabled(any, egui::Button::new(tr("Разложить заново всё")))
                .on_hover_text(tr(
                    "Вернуть все символы и провода на автоматические места и убрать с листов то, что положено вручную (подписи листов остаются)",
                ))
                .clicked()
            {
                view.confirm_reset = true;
            }
            ui.separator();
        }
        let busy = view.pdf_job.is_some();
        if ui
            .add_enabled(!busy && count > 0, egui::Button::new(tr("Собрать PDF")))
            .clicked()
        {
            if let Some(doc) = &app.doc {
                view.pdf_result = None;
                view.pdf_job = Some(start_pdf(
                    doc.project.clone(),
                    app.lib.clone(),
                    view.language(),
                    view.format,
                ));
            }
        }
        if busy {
            ui.spinner();
            use std::sync::mpsc::TryRecvError;
            match view.pdf_job.as_ref().map(|j| j.try_recv()) {
                Some(Ok(done)) => {
                    view.pdf_result = Some(done);
                    view.pdf_job = None;
                }
                Some(Err(TryRecvError::Disconnected)) => {
                    view.pdf_result = Some(Err(tr("PDF не собран").to_owned()));
                    view.pdf_job = None;
                }
                _ => {}
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        match &view.pdf_result {
            Some(Ok(path)) => {
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                ui.label(crate::i18n::fmt(tr("Готово: {0}"), &[&name]))
                    .on_hover_text(path.display().to_string());
            }
            Some(Err(e)) => {
                ui.colored_label(ui.visuals().warn_fg_color, e);
            }
            None => {}
        }
        if errors > 0 {
            ui.separator();
            let message = crate::i18n::fmt(tr("Ошибок в щите: {0}"), &[&errors]);
            ui.colored_label(ui.visuals().warn_fg_color, message);
        }
    });
}
