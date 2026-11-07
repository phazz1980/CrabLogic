use super::*;

use crate::debugview::DebugState;
use crate::simview::SimState;
use crab_core::{LocalValues, Logic, Project};

/// What every drawing pass of the canvas needs: the view transform, the painter and the
/// document, already resolved for this frame.
pub(super) struct Frame<'a> {
    pub(super) pal: Palette,
    pub(super) xf: Xf,
    pub(super) z: f32,
    pub(super) rect: Rect,
    pub(super) painter: &'a egui::Painter,
    pub(super) pointer: Option<Pos2>,
    pub(super) ctrl: bool,
    pub(super) project: &'a Project,
    pub(super) logic: &'a Logic,
    pub(super) lib: &'a crab_core::Library,
    pub(super) local: &'a LocalValues,
    pub(super) defs: &'a HashMap<&'a str, Option<&'a BlockDef>>,
    pub(super) bands: &'a [Band],
    pub(super) world_rect: &'a HashMap<&'a str, Rect>,
    pub(super) selected: Option<Selection>,
}

impl Frame<'_> {
    pub(super) fn font(&self, size: f32) -> FontId {
        FontId::proportional(size * self.z.clamp(0.5, 2.0))
    }

    pub(super) fn screen_rect(&self, id: &str) -> Option<Rect> {
        self.world_rect.get(id).map(|r| self.xf.rect(*r))
    }

    /// Screen position and type of a port.
    pub(super) fn port_screen(&self, r: &PortRef, output: bool) -> Option<(Pos2, SignalType)> {
        let block = self.logic.block(&r.block)?;
        let def = self.defs[r.block.as_str()]?;
        let ports: Vec<&PortDef> = if output {
            def.active_outputs(&block.params).collect()
        } else {
            def.active_inputs(&block.params).collect()
        };
        let i = ports.iter().position(|q| q.name == r.port)?;
        Some((
            port_pos(
                self.screen_rect(&r.block)?,
                kind_of(block, Some(def)),
                i,
                output,
                self.z,
            ),
            ports[i].ty,
        ))
    }
}

/// Live values of the running debug session or simulation.
#[derive(Clone, Copy)]
pub(super) struct Live<'a> {
    pub(super) dbg: Option<&'a DebugState>,
    pub(super) sim: Option<&'a SimState>,
}

impl Live<'_> {
    /// Live value of an output: from the board, else from the simulation.
    pub(super) fn output(&self, port: &PortRef) -> Option<(crab_debug::Value, bool)> {
        self.dbg
            .and_then(|d| d.output(port))
            .or_else(|| self.sim.and_then(|s| s.output(port)))
    }

    pub(super) fn editable(&self, port: &PortRef) -> bool {
        self.dbg
            .is_some_and(|d| d.table.output_index(port).is_some())
            || self.sim.is_some_and(|s| crate::simview::editable(s, port))
    }
}

/// What a click in the sheet headers asks for.
pub(super) struct SheetActions {
    pub(super) dialog: Option<u32>,
    pub(super) active: Option<u32>,
    pub(super) toggle: Option<u32>,
    pub(super) swap: Option<(u32, u32)>,
    pub(super) delete: Option<u32>,
    pub(super) add: bool,
}

/// Paints the sheets (header, frame, grid) and the "add sheet" strip.
pub(super) fn draw_sheets(ui: &mut Ui, f: &Frame, width: f32, current: u32) -> SheetActions {
    let Frame {
        pal,
        xf,
        z,
        rect,
        painter,
        project,
        bands,
        ..
    } = *f;
    let font = |size: f32| f.font(size);
    let mut sheet_dialog = None;
    let mut active_sheet = None;
    let mut toggle_sheet = None;
    let mut sheet_swap = None;
    let mut sheet_delete = None;
    for (bi, band) in bands.iter().enumerate() {
        let sheet_rect = xf.rect(Rect::from_min_size(
            Pos2::new(0.0, band.top),
            Vec2::new(width, band.height),
        ));
        painter.rect_filled(sheet_rect, 2.0, pal.sheet);
        let header = Rect::from_min_size(
            sheet_rect.min,
            Vec2::new(sheet_rect.width(), SHEET_HEADER * z),
        );
        painter.rect_filled(header, 2.0, pal.sheet_header);
        let info = project.layout.sheet(band.sheet);
        let background = info.is_some_and(|i| i.background);
        if background {
            // Violet tint and a frame: a background sheet is visible at a glance.
            let tint = Color32::from_rgb(150, 90, 220);
            painter.rect_filled(sheet_rect, 2.0, tint.gamma_multiply(0.10));
            painter.rect_filled(header, 2.0, tint.gamma_multiply(0.30));
            painter.rect_stroke(sheet_rect, 2.0, Stroke::new(2.0, tint), StrokeKind::Inside);
            painter.text(
                header.right_top() + Vec2::new(-86.0 * z, 4.0 * z),
                Align2::RIGHT_TOP,
                crate::i18n::tr("ФОНОВАЯ ЗАДАЧА · ядро 0"),
                font(10.0),
                tint,
            );
        }
        if info.is_some_and(|i| i.disabled) {
            // Grey tint: the sheet is left out of the check and the build.
            let grey = Color32::from_gray(140);
            painter.rect_filled(sheet_rect, 2.0, grey.gamma_multiply(0.18));
            painter.text(
                header.right_top() + Vec2::new(-86.0 * z, 16.0 * z),
                Align2::RIGHT_TOP,
                crate::i18n::tr("ПЛАТА ОТКЛЮЧЕНА"),
                font(10.0),
                grey,
            );
        }
        let lines = [
            (crate::i18n::fmt("Плата: {0}", &[&band.sheet]), pal.accent),
            (
                crate::i18n::fmt(
                    "Наименование: {0}",
                    &[&info.map_or("", |i| i.name.as_str())],
                ),
                pal.text,
            ),
            (
                crate::i18n::fmt(
                    "Комментарии: {0}",
                    &[&info.map_or("", |i| i.comment.as_str())],
                ),
                pal.weak,
            ),
        ];
        for (i, (text, color)) in lines.into_iter().enumerate() {
            painter.text(
                header.min + Vec2::new(28.0 * z, (4.0 + 11.5 * i as f32) * z),
                Align2::LEFT_TOP,
                text,
                font(10.0),
                color,
            );
        }
        let resp = ui.interact(
            header,
            Id::unique(("sheet_header", band.sheet)),
            Sense::click(),
        );
        if resp.double_clicked() {
            sheet_dialog = Some(band.sheet);
        }
        if resp.clicked() {
            active_sheet = Some(band.sheet);
        }
        resp.on_hover_text(crate::i18n::tr(
            "Двойной щелчок — название и комментарий платы",
        ));

        // The sheet being worked on stands out.
        if current == band.sheet {
            painter.rect_filled(header, 2.0, pal.accent.gamma_multiply(0.25));
            painter.rect_stroke(
                sheet_rect,
                2.0,
                Stroke::new(2.0, pal.accent),
                StrokeKind::Inside,
            );
        }

        // Move up / move down / delete, at the right edge of the header.
        let prev = bi.checked_sub(1).map(|i| bands[i].sheet);
        let next = bands.get(bi + 1).map(|b| b.sheet);
        let buttons = [
            (
                "✕",
                crate::i18n::tr("Удалить плату вместе с блоками (спросит подтверждение)"),
                None,
            ),
            ("▼", crate::i18n::tr("Опустить плату"), next),
            ("▲", crate::i18n::tr("Поднять плату"), prev),
        ];
        for (k, (label, hint, other)) in buttons.into_iter().enumerate() {
            let center = header.right_top()
                + Vec2::new(-(14.0 + 22.0 * k as f32) * z, SHEET_HEADER * 0.5 * z);
            let area = Rect::from_center_size(center, Vec2::splat(20.0 * z));
            let enabled = k == 0 || other.is_some();
            let r = ui.interact(
                area,
                Id::unique(("sheet_btn", band.sheet, k)),
                Sense::click(),
            );
            let color = if !enabled {
                pal.weak.gamma_multiply(0.4)
            } else if r.hovered() {
                pal.accent
            } else {
                pal.text
            };
            painter.text(center, Align2::CENTER_CENTER, label, font(12.0), color);
            if enabled && r.clicked() {
                match other {
                    Some(o) => sheet_swap = Some((band.sheet, o)),
                    None => sheet_delete = Some(band.sheet),
                }
            }
            r.on_hover_text(hint);
        }

        // Fold/unfold arrow at the left edge of the header.
        let arrow = Rect::from_center_size(
            header.min + Vec2::new(14.0 * z, SHEET_HEADER * 0.5 * z),
            Vec2::splat(20.0 * z),
        );
        let arrow_resp = ui.interact(
            arrow,
            Id::unique(("sheet_fold", band.sheet)),
            Sense::click(),
        );
        let c = arrow.center();
        let r = 5.0 * z;
        let tri = if band.collapsed {
            vec![
                c + Vec2::new(-r * 0.6, -r),
                c + Vec2::new(-r * 0.6, r),
                c + Vec2::new(r, 0.0),
            ]
        } else {
            vec![
                c + Vec2::new(-r, -r * 0.6),
                c + Vec2::new(r, -r * 0.6),
                c + Vec2::new(0.0, r),
            ]
        };
        let arrow_color = if arrow_resp.hovered() {
            pal.accent
        } else {
            pal.text
        };
        painter.add(egui::Shape::convex_polygon(tri, arrow_color, Stroke::NONE));
        if arrow_resp.clicked() {
            toggle_sheet = Some(band.sheet);
        }
        arrow_resp.on_hover_text(if band.collapsed {
            crate::i18n::tr("Развернуть плату")
        } else {
            crate::i18n::tr("Свернуть плату")
        });
        if band.collapsed {
            continue;
        }

        // Grid.
        if GRID * z > 8.0 {
            let body =
                Rect::from_min_max(Pos2::new(sheet_rect.min.x, header.max.y), sheet_rect.max)
                    .intersect(rect);
            let step = GRID * z;
            let mut x = sheet_rect.min.x + step;
            while x < body.max.x {
                if x > body.min.x {
                    painter.vline(x, body.y_range(), Stroke::new(1.0, pal.grid));
                }
                x += step;
            }
            let mut y = header.max.y + step;
            while y < body.max.y {
                if y > body.min.y {
                    painter.hline(body.x_range(), y, Stroke::new(1.0, pal.grid));
                }
                y += step;
            }
        }
    }
    // "Add sheet" strip above the first sheet.
    let add_rect = xf.rect(Rect::from_min_size(
        Pos2::ZERO,
        Vec2::new(width, ADD_SHEET_H),
    ));
    let add_resp = ui.interact(add_rect, Id::unique("add_sheet"), Sense::click());
    let add_color = if add_resp.hovered() {
        pal.accent
    } else {
        pal.weak
    };
    painter.rect_stroke(
        add_rect,
        2.0,
        Stroke::new(1.0, add_color),
        StrokeKind::Inside,
    );
    painter.text(
        add_rect.center(),
        Align2::CENTER_CENTER,
        crate::i18n::tr("+ Добавить плату"),
        font(11.0),
        add_color,
    );
    let add_sheet = add_resp.clicked();
    SheetActions {
        dialog: sheet_dialog,
        active: active_sheet,
        toggle: toggle_sheet,
        swap: sheet_swap,
        delete: sheet_delete,
        add: add_sheet,
    }
}

/// Everything the links pass found out: clicks and edits, plus labels drawn on top later.
pub(super) struct LinkOut {
    pub(super) clicked_link: Option<PortRef>,
    pub(super) wire_click: Option<PortRef>,
    pub(super) wire_edit: Option<(PortRef, Option<f32>)>,
    pub(super) wire_end: bool,
    pub(super) wire_break: Option<(PortRef, bool)>,
    /// Block a click on a broken link's stub leads to: the far end of the link.
    pub(super) wire_jump: Option<String>,
    /// Live values above the input end of each wire; drawn after the blocks to stay on top.
    pub(super) wire_labels: Vec<(Pos2, crab_debug::Value, bool, PortRef)>,
    pub(super) text_labels: Vec<(Pos2, String)>,
}

/// Draws every link and collects clicks on them.
pub(super) fn draw_links(ui: &mut Ui, f: &Frame, live: Live, click_pos: Option<Pos2>) -> LinkOut {
    let Frame {
        pal,
        xf,
        z,
        painter,
        project,
        logic,
        bands,
        ref selected,
        ..
    } = *f;
    let Live { dbg, sim } = live;
    let font = |size: f32| f.font(size);
    let port_screen = |r: &PortRef, output: bool| f.port_screen(r, output);
    let live_output = |port: &PortRef| live.output(port);
    let ctx = ui.ctx().clone();
    let mut clicked_link = None;
    let mut wire_click = None;
    let mut wire_edit: Option<(PortRef, Option<f32>)> = None;
    let mut wire_end = false;
    let mut wire_break: Option<(PortRef, bool)> = None;
    let mut wire_jump: Option<String> = None;
    let mut wire_labels = Vec::new();
    let mut text_labels = Vec::new();
    for link in &logic.links {
        let (Some((a, ty)), Some((b, _))) =
            (port_screen(&link.from, true), port_screen(&link.to, false))
        else {
            continue;
        };
        let sheet_of = |id: &str| project.layout.blocks.get(id).map(|p| p.sheet);
        let (from_sheet, to_sheet) = (sheet_of(&link.from.block), sheet_of(&link.to.block));
        // Links between sheets are broken into two labelled stubs.
        let key = crab_core::Layout::wire_key(&link.to);
        let route_cfg = project.layout.wires.get(&key).copied().unwrap_or_default();
        let cross = from_sheet != to_sheet;
        // Links inside a sheet can be broken by hand in the same way.
        let broken = cross || route_cfg.broken;
        let stub = 24.0 * z;
        let mut handle = None;
        let mut menu_areas: Vec<Rect> = Vec::new();
        let lines: Vec<Vec<Pos2>> = if broken {
            if !cross {
                menu_areas = vec![
                    Rect::from_two_pos(a, a + Vec2::new(stub, 0.0)).expand(5.0),
                    Rect::from_two_pos(b - Vec2::new(stub, 0.0), b).expand(5.0),
                ];
            }
            vec![
                vec![a, a + Vec2::new(stub, 0.0)],
                vec![b - Vec2::new(stub, 0.0), b],
            ]
        } else {
            let forward = is_forward(a, b, 12.0 * z);
            let band = bands.iter().find(|band| Some(band.sheet) == from_sheet);
            let bend = route_cfg.bend.zip(band).map(|(v, band)| {
                if forward {
                    xf.pos(Pos2::new(v, 0.0)).x
                } else {
                    xf.pos(Pos2::new(0.0, band.body_top() + v)).y
                }
            });
            let raw = route(a, b, 12.0 * z, bend);
            // The middle segment can be dragged to reroute the link.
            let seg = if forward {
                (raw[1], raw[2])
            } else {
                (raw[2], raw[3])
            };
            if let (Some(band), true) = (band, seg.0.distance(seg.1) > 4.0) {
                handle = Some((Rect::from_two_pos(seg.0, seg.1).expand(4.0), forward, *band));
            }
            menu_areas = vec![Rect::from_center_size(
                seg.0.lerp(seg.1, 0.5),
                Vec2::splat(12.0),
            )];
            vec![rounded(&raw, 6.0 * z)]
        };
        let is_sel = selected.as_ref() == Some(&Selection::Link(link.to.clone()));
        let color = signal_color(&pal, ty);
        // Online: logical lines light up when true and fade when false.
        // An inverted output feeds the inverted value to its links.
        let live = live_output(&link.from)
            .map(|(v, forced)| (flip(v, inverted_output(logic, &link.from)), forced));
        let (width, color) = match live {
            Some((crab_debug::Value::Bool(true), _)) => (2.6, pal.ok),
            Some((crab_debug::Value::Bool(false), _)) => (1.4, color.gamma_multiply(0.45)),
            _ => (1.6, color),
        };
        if let Some((v, forced)) = live {
            wire_labels.push((b, v, forced, link.from.clone()));
        }
        if let Some(text) = dbg
            .and_then(|d| d.text_output(&link.from))
            .or_else(|| sim.and_then(|s| s.text_output(&link.from)))
        {
            text_labels.push((b, text.to_owned()));
        }
        let stroke = Stroke::new(if is_sel { width + 1.4 } else { width }, color);
        if let Some((area, forward, band)) = handle {
            let resp = ui.interact(
                area,
                Id::unique(("wire", &link.to)),
                Sense::click_and_drag(),
            );
            if resp.hovered() || resp.dragged() {
                ctx.set_cursor_icon(if forward {
                    egui::CursorIcon::ResizeHorizontal
                } else {
                    egui::CursorIcon::ResizeVertical
                });
            }
            if resp.clicked() {
                wire_click = Some(link.to.clone());
            }
            if resp.double_clicked() {
                wire_edit = Some((link.to.clone(), None));
            } else if let (true, Some(p)) = (resp.dragged(), resp.interact_pointer_pos()) {
                let w = xf.world(p);
                let snap = |v: f32| (v / GRID).round() * GRID;
                let bend = if forward {
                    snap(w.x)
                } else {
                    snap(w.y - band.body_top())
                };
                wire_edit = Some((link.to.clone(), Some(bend)));
            }
            if resp.drag_stopped() {
                wire_end = true;
            }
            resp.on_hover_text(crate::i18n::tr(
                "Потяните, чтобы изменить путь; двойной щелчок — путь по умолчанию",
            ));
        }
        if !cross {
            for (i, area) in menu_areas.iter().enumerate() {
                let resp = ui.interact(*area, Id::unique(("wire-menu", &key, i)), Sense::click());
                resp.context_menu(|ui| {
                    let label = if route_cfg.broken {
                        crate::i18n::tr("Соединить линией")
                    } else {
                        crate::i18n::tr("Разорвать связь")
                    };
                    if ui.button(label).clicked() {
                        wire_break = Some((link.to.clone(), !route_cfg.broken));
                        ui.close();
                    }
                });
            }
        }
        for pts in &lines {
            painter.line(pts.clone(), stroke);
            if click_pos.is_some_and(|p| near_polyline(pts, p, 5.0)) {
                clicked_link = Some(link.to.clone());
            }
        }
        // Both stubs of a broken link are the same link: a click goes to its other end.
        let mut stub_areas = [
            Rect::from_two_pos(a, a + Vec2::new(stub, 0.0)).expand(5.0),
            Rect::from_two_pos(b - Vec2::new(stub, 0.0), b).expand(5.0),
        ];
        if cross {
            let sheet = |s: Option<u32>| s.map_or("?".to_owned(), |s| s.to_string());
            stub_areas[0] = stub_areas[0].union(painter.text(
                a + Vec2::new(stub + 3.0 * z, 0.0),
                Align2::LEFT_CENTER,
                crate::i18n::fmt(
                    "→ П{0}: {1}.{2}",
                    &[&sheet(to_sheet), &link.to.block, &link.to.port],
                ),
                font(10.0),
                color,
            ));
            stub_areas[1] = stub_areas[1].union(painter.text(
                b - Vec2::new(stub + 3.0 * z, 0.0),
                Align2::RIGHT_CENTER,
                crate::i18n::fmt(
                    "П{0}: {1}.{2} →",
                    &[&sheet(from_sheet), &link.from.block, &link.from.port],
                ),
                font(10.0),
                color,
            ));
        }
        if broken {
            for (i, (area, target)) in stub_areas.iter().zip([&link.to, &link.from]).enumerate() {
                let resp = ui.interact(*area, Id::unique(("wire-jump", &key, i)), Sense::click());
                if resp.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    wire_jump = Some(target.block.clone());
                }
                resp.on_hover_text(crate::i18n::fmt("Перейти к «{0}»", &[&target.block]));
            }
        }
    }
    LinkOut {
        clicked_link,
        wire_click,
        wire_edit,
        wire_end,
        wire_break,
        wire_jump,
        wire_labels,
        text_labels,
    }
}

/// What the blocks pass found out: selection, moves and clicks.
pub(super) struct BlockOut {
    pub(super) select: Option<Option<Selection>>,
    pub(super) moves: Vec<(String, Vec2)>,
    pub(super) drag_end: Option<String>,
    pub(super) open_dialog: Option<String>,
    pub(super) responses: Vec<(String, egui::Response)>,
    /// A block drag started on this block.
    pub(super) moving: Option<String>,
}

/// Draws the blocks. `errors` are the ids of blocks with a failed check; `wire_active` tells
/// whether a wire is being drawn (a block drag must not start then).
pub(super) fn draw_blocks(
    ui: &mut Ui,
    f: &Frame,
    live: Live,
    errors: &[&str],
    wire_active: bool,
) -> BlockOut {
    let Frame {
        pal,
        z,
        painter,
        pointer,
        ctrl,
        logic,
        lib,
        ref selected,
        ..
    } = *f;
    let Live { sim, .. } = live;
    let defs = f.defs;
    let font = |size: f32| f.font(size);
    let screen_rect = |id: &str| f.screen_rect(id);
    let mut moving = None;
    let mut select = None;
    let mut moves: Vec<(String, Vec2)> = Vec::new();
    let mut drag_end = None;
    let mut open_dialog = None;
    let mut responses: Vec<(String, egui::Response)> = Vec::new();
    let mut full_notes: Vec<FullNote> = Vec::new();
    for block in &logic.blocks {
        let id = block.id.as_str();
        let Some(r) = screen_rect(id) else { continue };
        let def = defs[id];
        let kind = kind_of(block, def);
        let resp = ui.interact(r, Id::unique(("block", id)), Sense::click_and_drag());
        let in_sel = selected.as_ref().is_some_and(|s| s.has_block(id));
        if resp.clicked() && ctrl {
            // Ctrl + click adds the block to the selection or removes it.
            let mut ids = selected
                .as_ref()
                .map(|s| s.blocks().to_vec())
                .unwrap_or_default();
            if in_sel {
                ids.retain(|b| b != id);
            } else {
                ids.push(id.to_owned());
            }
            select = Some(Selection::of_blocks(ids));
        } else if resp.clicked() || ((resp.secondary_clicked() || resp.drag_started()) && !in_sel) {
            // A right click on a selected block keeps the group for its menu.
            select = Some(Selection::of_blocks(vec![id.to_owned()]));
        }
        if resp.drag_started() {
            moving = Some(id.to_owned());
        }
        if resp.dragged() && !wire_active {
            // Dragging a block of a group moves the whole group.
            match selected.as_ref().filter(|s| in_sel && s.blocks().len() > 1) {
                Some(s) => {
                    for b in s.blocks() {
                        moves.push((b.clone(), resp.drag_delta() / z));
                    }
                }
                None => moves.push((id.to_owned(), resp.drag_delta() / z)),
            }
        }
        if resp.drag_stopped() {
            drag_end = Some(id.to_owned());
        }
        if resp.double_clicked() {
            open_dialog = Some(id.to_owned());
        }

        let is_sel = in_sel;
        let border = if errors.contains(&id) {
            Stroke::new(1.8, pal.error)
        } else if is_sel {
            Stroke::new(1.8, pal.accent)
        } else {
            Stroke::new(1.0, pal.block_border)
        };
        match kind {
            Kind::Tag { write, en } => {
                let fill = if write {
                    pal.block_header
                } else {
                    pal.tag_fill
                };
                painter.rect(
                    r,
                    CornerRadius::same((TAG_H * 0.5 * z) as u8),
                    fill,
                    border,
                    StrokeKind::Inside,
                );
                // The live value is drawn at the output pin, like for any block.
                let text = label(block, def);
                let center = if en {
                    Pos2::new(r.center().x, r.min.y + r.height() * 0.25)
                } else {
                    r.center()
                };
                painter.text(center, Align2::CENTER_CENTER, text, font(11.0), pal.text);
                if en {
                    let p = port_pos(r, kind, 1, false, z);
                    painter.text(
                        p + Vec2::new(6.0 * z, 0.0),
                        Align2::LEFT_CENTER,
                        "En",
                        font(9.5),
                        pal.text,
                    );
                    painter.circle_filled(
                        p,
                        PORT_R * z.sqrt(),
                        signal_color(&pal, SignalType::Bool),
                    );
                }
                if let Some(port) = def.and_then(|d| d.inputs.first().or(d.outputs.first())) {
                    let color = signal_color(&pal, port.ty);
                    painter.circle_filled(
                        port_pos(r, kind, 0, !write, z),
                        PORT_R * z.sqrt(),
                        color,
                    );
                    // Not connected yet: the other side is free too.
                    if !tag_linked(logic, lib, &block.id) {
                        painter.circle_filled(
                            port_pos(r, kind, 0, write, z),
                            PORT_R * z.sqrt(),
                            color,
                        );
                    }
                }
            }
            Kind::Box => {
                // A constant on an input is drawn with the ports, like a link.
                let fed = |port: &str| {
                    block.consts.contains_key(port)
                        || logic
                            .links
                            .iter()
                            .any(|l| l.to.block == id && l.to.port == port)
                };
                // Passwords are kept by the program: show them from there (masked).
                let filled = def
                    .filter(|d| d.params.iter().any(|p| p.local.is_some()))
                    .map(|d| {
                        let mut b = block.clone();
                        f.local.fill_block(d, &mut b);
                        b
                    });
                let block = filled.as_ref().unwrap_or(block);
                if let Some(note) = paint_box(painter, r, z, block, def, &pal, border, pointer, fed)
                {
                    full_notes.push(note);
                }
            }
        }
        // The virtual bench: the state of the pin an output block drives, inside the block
        // on the free (output) side.
        if let Some((_, state)) = sim.and_then(|s| s.output_pin(&block.id)) {
            let text = if block.kind == "pwm_out" {
                format!("{state:.0} %")
            } else if state != 0.0 {
                "HIGH".to_owned()
            } else {
                "LOW".to_owned()
            };
            let p = port_pos(r, Kind::Box, 0, true, z);
            painter.text(
                p - Vec2::new(6.0 * z, 0.0),
                Align2::RIGHT_CENTER,
                text,
                font(10.0),
                if state != 0.0 { pal.accent } else { pal.weak },
            );
        }
        // A selected block is tinted all over, so a group stands out from the sheet.
        if is_sel {
            painter.rect_filled(
                r,
                CornerRadius::same((3.0 * z) as u8),
                pal.accent.gamma_multiply(0.3),
            );
        }
        responses.push((id.to_owned(), resp));
    }
    // Cut parameter lines open in full, with a background, over everything else.
    for note in full_notes {
        let overlay = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::unique("block_note"),
        ));
        let galley = overlay.layout_no_wrap(note.text, font(9.0), pal.text);
        let bg = Rect::from_min_size(note.pos, galley.size()).expand2(Vec2::new(4.0, 1.0));
        overlay.rect(
            bg,
            CornerRadius::same(2),
            pal.block_fill,
            Stroke::new(1.0, pal.block_border),
            StrokeKind::Inside,
        );
        overlay.galley(note.pos, galley, pal.text);
    }
    BlockOut {
        select,
        moves,
        drag_end,
        open_dialog,
        responses,
        moving,
    }
}

/// A port on the screen: where a wire can start or end.
pub(super) struct Hit {
    pub(super) port: PortRef,
    pub(super) output: bool,
    pub(super) ty: SignalType,
    pub(super) pos: Pos2,
}

/// What the ports pass found out.
pub(super) struct PortOut {
    pub(super) hits: Vec<Hit>,
    pub(super) port_menus: Vec<(PortRef, bool, SignalType, egui::Response)>,
    /// Value labels a click on which opens an editor: (label, output behind it).
    pub(super) value_hits: Vec<(Rect, PortRef)>,
    /// A wire drag that started on one of the ports.
    pub(super) wire_started: Option<WireDrag>,
}

/// Draws the ports, their constants and live values; starts wire drags.
pub(super) fn draw_ports(ui: &mut Ui, f: &Frame, live: Live) -> PortOut {
    let Frame {
        pal,
        xf,
        z,
        painter,
        logic,
        lib,
        defs,
        ..
    } = *f;
    let Live { dbg, sim } = live;
    let font = |size: f32| f.font(size);
    let screen_rect = |id: &str| f.screen_rect(id);
    let live_output = |port: &PortRef| live.output(port);
    let editable = |port: &PortRef| live.editable(port);
    let mut wire_started = None;
    let mut value_hits: Vec<(Rect, PortRef)> = Vec::new();
    let mut hits: Vec<Hit> = Vec::new();
    let mut port_menus: Vec<(PortRef, bool, SignalType, egui::Response)> = Vec::new();
    for block in &logic.blocks {
        let (Some(r), Some(def)) = (screen_rect(&block.id), defs[block.id.as_str()]) else {
            continue;
        };
        let kind = kind_of(block, Some(def));
        // An unconnected variable block offers both sides; the link decides
        // whether it reads (output `out`) or writes (input `in`).
        let write = block.kind == VAR_SET;
        // With the write enable input the block only writes: no output side.
        let tag_port = (block.var_name().is_some()
            && !tag_linked(logic, lib, &block.id)
            && kind
                != (Kind::Tag {
                    write: true,
                    en: true,
                }))
        .then(|| def.inputs.first().or(def.outputs.first()))
        .flatten()
        .map(|q| {
            let q = PortDef {
                name: if write { "out" } else { "in" }.to_owned(),
                ..q.clone()
            };
            (q, write)
        });
        if let Some((q, output)) = &tag_port {
            let pos = port_pos(r, kind, 0, *output, z);
            painter.circle_filled(pos, PORT_R * xf.view.zoom.sqrt(), signal_color(&pal, q.ty));
        }
        let ports = def
            .active_inputs(&block.params)
            .enumerate()
            .map(|(i, q)| (i, q, false))
            .chain(
                def.active_outputs(&block.params)
                    .enumerate()
                    .map(|(i, q)| (i, q, true)),
            )
            .map(|(i, q, output)| (i, q, output, true))
            .chain(tag_port.as_ref().map(|(q, output)| (0, q, *output, false)));
        for (i, q, output, real) in ports {
            let pos = port_pos(r, kind, i, output, z);
            let area = Rect::from_center_size(pos, Vec2::splat(10.0));
            let resp = ui.interact(
                area,
                Id::unique(("port", &block.id, &q.name)),
                Sense::click_and_drag(),
            );
            let inverted = real
                && if output {
                    &block.inverted
                } else {
                    &block.inverted_inputs
                }
                .contains(&q.name);
            if inverted {
                // Small circle on the outer side of the port.
                let rr = 3.2 * z.sqrt();
                let side = if output { rr } else { -rr };
                painter.circle(
                    pos + Vec2::new(side, 0.0),
                    rr,
                    pal.sheet,
                    Stroke::new(1.2, signal_color(&pal, q.ty)),
                );
            }
            if let Some(v) = (!output && real)
                .then(|| block.consts.get(&q.name))
                .flatten()
            {
                // Constant on an input: its value on a short stub.
                let start = pos - Vec2::new(14.0 * z, 0.0);
                painter.line_segment([start, pos], Stroke::new(1.4, pal.text));
                painter.text(
                    start - Vec2::new(2.0 * z, 0.0),
                    Align2::RIGHT_CENTER,
                    value_text(v),
                    font(9.5),
                    pal.text,
                );
            }
            if let (true, true, Kind::Tag { write: false, .. }, Some(var)) =
                (output, real, kind, block.var_name())
            {
                if let Some((v, forced)) = dbg.and_then(|d| d.var(var)) {
                    debugview::value_label(painter, &pal, pos, v, forced, z, Place::Right);
                } else if let Some((v, forced)) = sim.and_then(|s| s.var(logic, var)) {
                    let rect =
                        debugview::value_label(painter, &pal, pos, v, forced, z, Place::Right);
                    value_hits.push((rect, PortRef::new(&block.id, &q.name)));
                } else if let Some(text) = dbg.and_then(|d| d.text_var(var)) {
                    debugview::text_label(painter, &pal, pos, text, z, Place::Right);
                }
            }
            if output && kind == Kind::Box {
                let port = PortRef::new(&block.id, &q.name);
                if let Some((v, forced)) = live_output(&port) {
                    let v = flip(v, inverted);
                    let rect =
                        debugview::value_label(painter, &pal, pos, v, forced, z, Place::Right);
                    if editable(&port) {
                        value_hits.push((rect, port.clone()));
                    }
                } else if let Some(text) = dbg.and_then(|d| d.text_output(&port)) {
                    debugview::text_label(painter, &pal, pos, text, z, Place::Right);
                }
            }
            if real {
                port_menus.push((PortRef::new(&block.id, &q.name), output, q.ty, resp.clone()));
            }
            if resp.hovered() || resp.dragged() {
                painter.circle_stroke(
                    pos,
                    PORT_R * 2.0,
                    Stroke::new(1.4, signal_color(&pal, q.ty)),
                );
            }
            let port = PortRef::new(&block.id, &q.name);
            if resp.drag_started() {
                wire_started = Some(WireDrag {
                    port: port.clone(),
                    from_output: output,
                });
            }
            hits.push(Hit {
                port,
                output,
                ty: q.ty,
                pos,
            });
        }
    }
    PortOut {
        hits,
        port_menus,
        value_hits,
        wire_started,
    }
}
