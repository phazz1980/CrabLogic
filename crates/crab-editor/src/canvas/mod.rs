//! Working area: numbered sheets stacked top to bottom.
//!
//! `layout.toml` stores block positions relative to their sheet; here the sheets
//! are laid out one under another and share one pan and zoom.

use std::collections::{HashMap, HashSet};

use crab_core::vars::{VAR_GET, VAR_SET};
use crab_core::{
    Block, BlockDef, BlockPos, Logic, ParamValue, PortDef, PortRef, Severity, SignalType,
};
use eframe::egui::containers::menu::{MenuConfig, SubMenuButton};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};

use crate::app::{EditorApp, LogLevel, Selection};
use crate::debugview::{self, Place};
use crate::dialogs::Dialog;
use crate::theme::Palette;

mod geometry;
mod menus;
mod paint;
#[cfg(test)]
mod tests;

pub use self::menus::sheet_delete_dialog;
pub use self::paint::block_preview;
use self::{geometry::*, menus::*, paint::*};

// Sizes are multiples of the grid so that every port sits on a grid line.
const HEAD_H: f32 = 20.0;
const ROW_H: f32 = 20.0;
const TAG_H: f32 = 20.0;
const PORT_R: f32 = 3.0;
const GRID: f32 = 10.0;
const SHEET_HEADER: f32 = 40.0;
const SHEET_MIN_BODY: f32 = 300.0;
const SHEET_MIN_W: f32 = 1200.0;
const SHEET_GAP: f32 = 14.0;
const SHEET_PAD: f32 = 60.0;
const ADD_SHEET_H: f32 = 28.0;

/// Payload dragged from the library or the variable list onto a sheet.
#[derive(Debug, Clone)]
pub enum DragItem {
    Block(String),
    /// Variable: the side the first link comes to decides read or write.
    Var(String),
    /// Setting or port of a service block (Wi-Fi...) taken out to the board.
    Setting {
        id: String,
        name: String,
        ty: SignalType,
        feed: bool,
    },
    /// MQTT topic block (id) from the side panel: it is drawn on the sheet at the drop point.
    Topic(String),
}

/// Drag source that also reports clicks and double clicks. `Ui::dnd_drag_source` puts a
/// drag-only area over its contents, and egui then gives clicks on it to nobody.
pub fn drag_source(
    ui: &mut egui::Ui,
    id: egui::Id,
    item: DragItem,
    add: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    if ui.ctx().is_being_dragged(id) {
        // egui paints the dragged copy under the pointer and sets the payload.
        return ui.dnd_drag_source(id, item, add).response;
    }
    let rect = add(ui).rect;
    ui.interact(rect, id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::Grab)
}

#[derive(Debug, Clone, Copy)]
struct View {
    pan: Vec2,
    zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            pan: Vec2::new(16.0, 12.0),
            zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
struct WireDrag {
    port: PortRef,
    from_output: bool,
}

#[derive(Debug, Default)]
pub struct CanvasState {
    view: View,
    wire: Option<WireDrag>,
    /// Block being dragged: sheet heights ignore it, so it can move to another sheet.
    moving: Option<String>,
    /// Block to scroll into view on the next frame.
    pub focus: Option<String>,
    /// A moved sheet and the screen offset (from the canvas top) its header had: the
    /// canvas scrolls on the next frame so the sheet stays where the user clicked.
    follow_sheet: Option<(u32, f32)>,
    /// Screen corner where a selection frame (left drag) started.
    band: Option<Pos2>,
    /// Canvas point under the mouse: where pasted blocks go.
    hover: Option<BlockPos>,
    /// Canvas point of the last right click.
    menu_at: Option<BlockPos>,
    /// Text of the constant being typed in the menu of an input.
    const_text: String,
    /// Sheets folded to their header.
    collapsed: HashSet<u32>,
}

impl CanvasState {
    /// Where pasted blocks go: the spot of the last right click if «Вставить» was
    /// chosen in a menu, else the point under the mouse.
    pub fn paste_pos(&mut self) -> Option<BlockPos> {
        self.menu_at.take().or(self.hover)
    }
}

pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    // A block asked to be shown must not sit in a folded sheet.
    if let (Some(id), Some(doc)) = (&app.canvas.focus, &app.doc) {
        if let Some(pos) = doc.project.layout.blocks.get(id) {
            app.canvas.collapsed.remove(&pos.sheet);
        }
    }
    let Some(doc) = &app.doc else {
        ui.centered_and_justified(|ui| {
            ui.label(crate::i18n::tr(
                "Откройте проект: Файл → Открыть, или создайте новый.",
            ));
        });
        return;
    };
    let pal = app.theme.palette;
    let project = &doc.project;
    let logic = &project.logic;

    // Geometry in world space.
    let defs: HashMap<&str, Option<&BlockDef>> = logic
        .blocks
        .iter()
        .map(|b| (b.id.as_str(), app.lib.resolve(logic, b).map(|l| &l.def)))
        .collect();
    let sizes: HashMap<&str, Vec2> = logic
        .blocks
        .iter()
        .map(|b| (b.id.as_str(), size(b, defs[b.id.as_str()])))
        .collect();
    let bands = bands(app, &sizes, ui.input(|i| i.modifiers.shift));
    let band_of = |sheet: u32| bands.iter().find(|b| b.sheet == sheet).copied();
    let width = project
        .layout
        .blocks
        .iter()
        .map(|(id, p)| p.x + sizes.get(id.as_str()).map_or(0.0, |s| s.x) + SHEET_PAD)
        .fold(SHEET_MIN_W, f32::max);
    let world_rect: HashMap<&str, Rect> = logic
        .blocks
        .iter()
        .filter_map(|b| {
            // Service blocks (Wi-Fi, web interface) live in the left panel only;
            // a topic is drawn once it has been dragged onto a sheet.
            let pos = project.layout.blocks.get(&b.id);
            if defs[b.id.as_str()]
                .is_some_and(|d| d.info.hidden && !(d.info.pullable && pos.is_some()))
            {
                return None;
            }
            let pos = pos?;
            let band = band_of(pos.sheet).filter(|b| !b.collapsed)?;
            let min = Pos2::new(pos.x, band.body_top() + pos.y);
            Some((
                b.id.as_str(),
                Rect::from_min_size(min, sizes[b.id.as_str()]),
            ))
        })
        .collect();

    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let bg = ui.interact(rect, Id::unique("canvas_bg"), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let mut view = app.canvas.view;

    // The view moves only to a block out of sight: a click in the tree must not
    // make the board jump.
    if let Some(id) = app.canvas.focus.take() {
        if let Some(r) = world_rect.get(id.as_str()) {
            let shown = Xf {
                origin: rect.min,
                view,
            }
            .rect(*r);
            if !rect.contains_rect(shown) {
                view.pan = rect.size() * 0.5 - r.center().to_vec2() * view.zoom;
            }
        }
    }
    if let Some((sheet, offset)) = app.canvas.follow_sheet.take() {
        if let Some(band) = bands.iter().find(|b| b.sheet == sheet) {
            view.pan.y = offset - band.top * view.zoom;
        }
    }
    let xf = Xf {
        origin: rect.min,
        view,
    };
    let z = view.zoom;
    let font = |size: f32| FontId::proportional(size * z.clamp(0.5, 2.0));
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let ctrl = ui.input(|i| i.modifiers.command);

    // Sheets.
    painter.rect_filled(rect, 0.0, pal.canvas);
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
        if app.sheet == band.sheet {
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

    // Ports in screen space.
    let errors: Vec<&str> = doc
        .report
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .filter_map(|d| d.block.as_deref())
        .collect();
    let screen_rect = |id: &str| world_rect.get(id).map(|r| xf.rect(*r));
    let port_screen = |r: &PortRef, output: bool| -> Option<(Pos2, SignalType)> {
        let block = logic.block(&r.block)?;
        let def = defs[r.block.as_str()]?;
        let ports: Vec<&PortDef> = if output {
            def.active_outputs(&block.params).collect()
        } else {
            def.active_inputs(&block.params).collect()
        };
        let i = ports.iter().position(|q| q.name == r.port)?;
        Some((
            port_pos(
                screen_rect(&r.block)?,
                kind_of(block, Some(def)),
                i,
                output,
                z,
            ),
            ports[i].ty,
        ))
    };

    // Links.
    let selected = app.selection.clone();
    let dbg = app.debug.as_ref();
    let sim = app.sim.as_ref();
    // Live value of an output: from the board, else from the simulation.
    let live_output = |port: &crab_core::PortRef| {
        dbg.and_then(|d| d.output(port))
            .or_else(|| sim.and_then(|s| s.output(port)))
    };
    let click_pos = bg.clicked().then(|| bg.interact_pointer_pos()).flatten();
    let mut clicked_link = None;
    let mut wire_click = None;
    let mut wire_edit: Option<(crab_core::PortRef, Option<f32>)> = None;
    let mut wire_end = false;
    let mut wire_break: Option<(crab_core::PortRef, bool)> = None;
    // Block a click on a broken link's stub leads to: the far end of the link.
    let mut wire_jump: Option<String> = None;
    let ctx = ui.ctx().clone();
    // Live values above the input end of each wire; drawn after the blocks to stay on top.
    let mut wire_labels = Vec::new();
    // Value labels a click on which opens an editor: (label, output behind it).
    let mut value_hits: Vec<(Rect, PortRef)> = Vec::new();
    let editable = |port: &PortRef| {
        dbg.is_some_and(|d| d.table.output_index(port).is_some())
            || sim.is_some_and(|s| crate::simview::editable(s, port))
    };
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

    // Blocks.
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
            app.canvas.moving = Some(id.to_owned());
        }
        if resp.dragged() && app.canvas.wire.is_none() {
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
                    if !tag_linked(logic, &app.lib, &block.id) {
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
                        app.local.fill_block(d, &mut b);
                        b
                    });
                let block = filled.as_ref().unwrap_or(block);
                if let Some(note) =
                    paint_box(&painter, r, z, block, def, &pal, border, pointer, fed)
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

    // Ports: drag a wire from any port.
    struct Hit {
        port: PortRef,
        output: bool,
        ty: SignalType,
        pos: Pos2,
    }
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
            && !tag_linked(logic, &app.lib, &block.id)
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
                    debugview::value_label(&painter, &pal, pos, v, forced, z, Place::Right);
                } else if let Some((v, forced)) = sim.and_then(|s| s.var(logic, var)) {
                    let rect =
                        debugview::value_label(&painter, &pal, pos, v, forced, z, Place::Right);
                    value_hits.push((rect, PortRef::new(&block.id, &q.name)));
                } else if let Some(text) = dbg.and_then(|d| d.text_var(var)) {
                    debugview::text_label(&painter, &pal, pos, text, z, Place::Right);
                }
            }
            if output && kind == Kind::Box {
                let port = PortRef::new(&block.id, &q.name);
                if let Some((v, forced)) = live_output(&port) {
                    let v = flip(v, inverted);
                    let rect =
                        debugview::value_label(&painter, &pal, pos, v, forced, z, Place::Right);
                    if editable(&port) {
                        value_hits.push((rect, port.clone()));
                    }
                } else if let Some(text) = dbg.and_then(|d| d.text_output(&port)) {
                    debugview::text_label(&painter, &pal, pos, text, z, Place::Right);
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
                app.canvas.wire = Some(WireDrag {
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

    for (anchor, text) in text_labels {
        debugview::text_label(&painter, &pal, anchor, &text, z, Place::Left);
    }
    for (anchor, v, forced, port) in wire_labels {
        let rect = debugview::value_label(&painter, &pal, anchor, v, forced, z, Place::Left);
        if editable(&port) {
            value_hits.push((rect, port));
        }
    }
    // A left click on a value: switch a bool, type a number.
    for (n, (rect, port)) in value_hits.into_iter().enumerate() {
        let resp = ui.interact(rect, Id::unique(("value_label", n)), Sense::click());
        let index = app.debug.as_ref().and_then(|d| d.table.output_index(&port));
        let mut debug = app.debug.take();
        let mut sim = app.sim.take();
        egui::Popup::menu(&resp).show(|ui| {
            if let (Some(d), Some(i)) = (&mut debug, index) {
                debugview::value_menu(ui, d, i);
            }
            if let Some(s) = &mut sim {
                crate::simview::port_menu(ui, s, &port);
            }
        });
        app.debug = debug;
        app.sim = sim;
    }

    // Wire being drawn.
    let released = ui.input(|i| i.pointer.any_released());
    let mut connect = None;
    if let Some(wire) = &app.canvas.wire {
        let start = hits
            .iter()
            .find(|h| h.port == wire.port && h.output == wire.from_output);
        if let (Some(h), Some(p)) = (start, pointer) {
            let (a, b) = if wire.from_output {
                (h.pos, p)
            } else {
                (p, h.pos)
            };
            let pts = rounded(&route(a, b, 12.0 * z, None), 6.0 * z);
            painter.line(
                pts,
                Stroke::new(1.6, signal_color(&pal, h.ty).gamma_multiply(0.7)),
            );
        }
        if released {
            let target = pointer.and_then(|p| {
                hits.iter()
                    .filter(|h| h.output != wire.from_output && h.pos.distance(p) < 10.0)
                    .min_by(|a, b| a.pos.distance(p).total_cmp(&b.pos.distance(p)))
            });
            if let Some(h) = target {
                connect = Some(if wire.from_output {
                    (wire.port.clone(), h.port.clone())
                } else {
                    (h.port.clone(), wire.port.clone())
                });
            }
            app.canvas.wire = None;
        }
    }

    // Drop from the library or the variable list.
    let dropped = bg.dnd_release_payload::<DragItem>();
    let drop_at = |p: Pos2| -> Option<BlockPos> {
        let w = xf.world(p);
        let open = || bands.iter().filter(|b| !b.collapsed);
        let band = open()
            .find(|b| w.y >= b.top && w.y < b.top + b.height)
            .or(open().next_back())?;
        Some(BlockPos {
            sheet: band.sheet,
            x: (w.x / GRID).round().max(0.0) * GRID,
            y: ((w.y - band.body_top()) / GRID).round().max(0.0) * GRID,
        })
    };
    app.canvas.hover = pointer.filter(|p| rect.contains(*p)).and_then(drop_at);
    if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary))
        && app.canvas.hover.is_some()
    {
        app.canvas.menu_at = app.canvas.hover;
    }

    // Selection frame: left drag on empty space.
    let mut framed = None;
    if bg.drag_started_by(egui::PointerButton::Primary) && app.canvas.wire.is_none() {
        app.canvas.band = bg.interact_pointer_pos();
    }
    if let (Some(a), Some(b)) = (app.canvas.band, pointer) {
        let frame = Rect::from_two_pos(a, b);
        painter.rect(
            frame,
            CornerRadius::ZERO,
            pal.accent.gamma_multiply(0.12),
            Stroke::new(1.0, pal.accent),
            StrokeKind::Inside,
        );
        if bg.drag_stopped() || released {
            let ids: Vec<String> = logic
                .blocks
                .iter()
                .filter(|bl| screen_rect(&bl.id).is_some_and(|r| frame.intersects(r)))
                .map(|bl| bl.id.clone())
                .collect();
            framed = Some(Selection::of_blocks(ids));
            app.canvas.band = None;
        }
    }
    let drop = dropped.and_then(|item| Some(((*item).clone(), drop_at(pointer?)?)));
    // Where a moved block ends up: the sheet under its top-left corner, or the sheet it
    // already stands on when Shift is held.
    let shift = ui.input(|i| i.modifiers.shift);
    let landing = drag_end.as_ref().and_then(|id| {
        let r = world_rect.get(id.as_str())?;
        let open = || bands.iter().filter(|b| !b.collapsed);
        let home = app
            .doc
            .as_ref()
            .and_then(|d| d.project.layout.blocks.get(id.as_str()))
            .map(|p| p.sheet)
            .filter(|_| shift);
        let band = home
            .and_then(|sheet| open().find(|b| b.sheet == sheet))
            .or_else(|| {
                open()
                    .find(|b| r.min.y >= b.top && r.min.y < b.top + b.height)
                    .or(if r.min.y < 0.0 {
                        open().next()
                    } else {
                        open().next_back()
                    })
            })?;
        Some(BlockPos {
            sheet: band.sheet,
            x: (r.min.x / GRID).round().max(0.0) * GRID,
            y: ((r.min.y - band.body_top()) / GRID).round().max(0.0) * GRID,
        })
    });

    // Pan and zoom.
    // Pan with the right or middle button; the left one draws a selection frame.
    let panning =
        bg.dragged_by(egui::PointerButton::Secondary) || bg.dragged_by(egui::PointerButton::Middle);
    if panning && app.canvas.wire.is_none() && app.canvas.band.is_none() {
        view.pan += bg.drag_delta();
    }
    if ui.rect_contains_pointer(rect) {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        if zoom != 1.0 {
            let anchor = pointer.unwrap_or(rect.center());
            let world = xf.world(anchor);
            view.zoom = (view.zoom * zoom).clamp(0.3, 3.0);
            view.pan = anchor - rect.min - world.to_vec2() * view.zoom;
        } else {
            view.pan += scroll;
        }
    }
    // Keep the sheets on screen: no empty space beyond their edges.
    let height = bands.last().map_or(0.0, |b| b.top + b.height);
    let content = Vec2::new(width, height) * view.zoom;
    let margin = Vec2::new(16.0, 12.0);
    let lo = rect.size() - margin - content;
    view.pan.x = view.pan.x.min(margin.x).max(lo.x.min(margin.x));
    view.pan.y = view.pan.y.min(margin.y).max(lo.y.min(margin.y));
    app.canvas.view = view;

    // Apply changes (the document was borrowed for drawing above).
    if let Some(id) = wire_jump {
        app.focus_block(&id);
    } else if bg.clicked() {
        app.select(clicked_link.map(Selection::Link));
    }
    if let Some(to) = wire_click {
        app.select(Some(Selection::Link(to)));
    }
    if let Some(s) = framed {
        app.select(s);
    }
    if let Some(s) = select {
        if let Some(Selection::Block(id)) = &s {
            if let Some(p) = app
                .doc
                .as_ref()
                .and_then(|d| d.project.layout.blocks.get(id))
            {
                app.sheet = p.sheet;
            }
        }
        app.select(s);
    }
    if let Some(s) = active_sheet {
        app.sheet = s;
    }
    let lib = app.lib.clone();
    let mut problems = Vec::new();
    let mut added = None;
    if let Some(doc) = &mut app.doc {
        let group = moves.len() > 1;
        for (id, delta) in moves {
            if let Some(&pos) = doc.project.layout.blocks.get(&id) {
                let moved = BlockPos {
                    x: pos.x + delta.x,
                    y: pos.y + delta.y,
                    ..pos
                };
                let key = if group {
                    "move:group".to_owned()
                } else {
                    format!("move:{id}")
                };
                let _ = doc.edit(&lib, Some(key), |p| p.move_block(&id, moved));
            }
        }
        let group_end = drag_end.as_ref().and_then(|id| {
            let ids = app.selection.as_ref()?.blocks();
            (ids.len() > 1 && ids.contains(id)).then(|| ids.to_vec())
        });
        if let Some(ids) = group_end {
            // A group stays on its sheets; positions snap to the grid.
            let _ = doc.edit(&lib, Some("move:group".to_owned()), |p| {
                for id in &ids {
                    if let Some(&pos) = p.layout.blocks.get(id) {
                        let snap = |v: f32| (v / GRID).round().max(0.0) * GRID;
                        p.move_block(
                            id,
                            BlockPos {
                                x: snap(pos.x),
                                y: snap(pos.y),
                                ..pos
                            },
                        )?;
                    }
                }
                Ok::<_, crab_core::EditError>(())
            });
            doc.end_edit();
        } else if let (Some(id), Some(pos)) = (&drag_end, landing) {
            let _ = doc.edit(&lib, Some(format!("move:{id}")), |p| p.move_block(id, pos));
            doc.end_edit();
        }
        if let Some((to, bend)) = wire_edit {
            let key = format!("wire:{}.{}", to.block, to.port);
            let _ = doc.edit(&lib, Some(key), |p| {
                p.set_wire_bend(&to, bend);
                Ok::<_, crab_core::EditError>(())
            });
        }
        if wire_end {
            doc.end_edit();
        }
        if let Some((to, broken)) = wire_break {
            let _ = doc.edit(&lib, None, |p| {
                p.set_wire_broken(&to, broken);
                Ok::<_, crab_core::EditError>(())
            });
        }
        if let Some((from, to)) = connect {
            if let Err(e) = doc.edit(&lib, None, |p| p.connect(&lib, from, to)) {
                problems.push(e.to_string());
            }
        }
        if let Some((item, pos)) = drop {
            let result = match item {
                DragItem::Block(kind) => doc.edit(&lib, None, |p| p.add_block(&lib, &kind, pos)),
                DragItem::Var(name) => doc.edit(&lib, None, |p| p.add_var_block(&name, false, pos)),
                DragItem::Setting { id, name, ty, feed } => doc.edit(&lib, None, |p| {
                    crate::netpanel::pull_out(p, &lib, &id, &name, ty, feed, pos)
                }),
                // A tag already on the board gets one more copy; otherwise the
                // block itself is placed (or moved).
                DragItem::Topic(id) => doc.edit(&lib, None, |p| {
                    let tag = p.logic.tag_of(&id).to_owned();
                    let is_tag = p.is_widget(&tag)
                        || p.logic.block(&tag).is_some_and(|b| {
                            lib.resolve(&p.logic, b).is_some_and(|l| {
                                l.def
                                    .params
                                    .iter()
                                    .any(|d| d.kind == crab_core::ParamKind::Pin)
                            })
                        });
                    let placed = std::iter::once(tag.as_str())
                        .chain(p.logic.tag_copies(&tag))
                        .any(|b| p.layout.blocks.contains_key(b));
                    // A widget never stands on a sheet itself, only its images do.
                    if is_tag && (placed || p.is_widget(&tag)) {
                        p.add_tag_copy(&tag, pos)
                    } else {
                        p.layout.blocks.insert(id.clone(), pos);
                        Ok(id)
                    }
                }),
            };
            match result {
                Ok(id) => added = Some(id),
                Err(e) => problems.push(e.to_string()),
            }
        }
    }
    if drag_end.is_some() {
        app.canvas.moving = None;
    }
    if let Some(sheet) = toggle_sheet {
        if !app.canvas.collapsed.remove(&sheet) {
            app.canvas.collapsed.insert(sheet);
        }
    }
    if let Some((a, b)) = sheet_swap {
        if let Some(doc) = &mut app.doc {
            let _ = doc.edit(&lib, None, |p| {
                p.swap_sheets(a, b);
                Ok::<_, crab_core::EditError>(())
            });
        }
        let (ea, eb) = (app.extra_sheets.remove(&a), app.extra_sheets.remove(&b));
        if ea {
            app.extra_sheets.insert(b);
        }
        if eb {
            app.extra_sheets.insert(a);
        }
        let (ca, cb) = (
            app.canvas.collapsed.remove(&a),
            app.canvas.collapsed.remove(&b),
        );
        if ca {
            app.canvas.collapsed.insert(b);
        }
        if cb {
            app.canvas.collapsed.insert(a);
        }
        // The focus and the view follow the moved sheet.
        app.sheet = b;
        if let Some(band) = bands.iter().find(|x| x.sheet == a) {
            let offset = xf.pos(Pos2::new(0.0, band.top)).y - rect.min.y;
            app.canvas.follow_sheet = Some((b, offset));
            ui.ctx().request_repaint();
        }
    }
    if let Some(sheet) = sheet_delete {
        app.sheet_delete = Some(sheet);
    }
    if add_sheet {
        let next = app.sheets().last().copied().unwrap_or(1) + 1;
        app.extra_sheets.insert(next);
        app.sheet = next;
    }
    if let Some(id) = added {
        app.select(Some(Selection::Block(id)));
    }
    for e in problems {
        app.info(LogLevel::Error, e);
    }
    if let Some(id) = open_dialog {
        app.dialog = Dialog::for_block(app, &id);
    }
    if let Some(sheet) = sheet_dialog {
        app.dialog = Some(Dialog::for_sheet(app, sheet));
    }
    for (id, resp) in responses {
        block_context_menu(app, &id, &resp);
    }
    for (port, output, ty, resp) in port_menus {
        port_context_menu(app, &port, output, ty, &resp);
    }
    bg.context_menu(|ui| {
        clipboard_items(app, ui);
    });
}
