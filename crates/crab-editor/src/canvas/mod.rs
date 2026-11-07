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

mod apply;
mod draw;
mod geometry;
mod menus;
mod paint;
#[cfg(test)]
mod tests;

pub use self::menus::sheet_delete_dialog;
pub use self::paint::block_preview;
use self::{apply::*, draw::*, geometry::*, menus::*, paint::*};

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
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let ctrl = ui.input(|i| i.modifiers.command);
    let frame = Frame {
        pal,
        xf,
        z,
        rect,
        painter: &painter,
        pointer,
        ctrl,
        project,
        logic,
        lib: &app.lib,
        local: &app.local,
        defs: &defs,
        bands: &bands,
        world_rect: &world_rect,
        selected: app.selection.clone(),
    };

    // Sheets.
    painter.rect_filled(rect, 0.0, pal.canvas);
    let SheetActions {
        dialog: sheet_dialog,
        active: active_sheet,
        toggle: toggle_sheet,
        swap: sheet_swap,
        delete: sheet_delete,
        add: add_sheet,
    } = draw_sheets(ui, &frame, width, app.sheet);

    // Ports in screen space.
    let errors: Vec<&str> = doc
        .report
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .filter_map(|d| d.block.as_deref())
        .collect();
    let screen_rect = |id: &str| world_rect.get(id).map(|r| xf.rect(*r));

    // Links.
    let live = Live {
        dbg: app.debug.as_ref(),
        sim: app.sim.as_ref(),
    };
    let editable = |port: &PortRef| live.editable(port);
    let click_pos = bg.clicked().then(|| bg.interact_pointer_pos()).flatten();
    let LinkOut {
        clicked_link,
        wire_click,
        wire_edit,
        wire_end,
        wire_break,
        wire_jump,
        wire_labels,
        text_labels,
    } = draw_links(ui, &frame, live, click_pos);

    // Blocks.
    let BlockOut {
        select,
        moves,
        drag_end,
        open_dialog,
        responses,
        moving,
    } = draw_blocks(ui, &frame, live, &errors, app.canvas.wire.is_some());
    if moving.is_some() {
        app.canvas.moving = moving;
    }

    // Ports: drag a wire from any port.
    let PortOut {
        hits,
        port_menus,
        mut value_hits,
        wire_started,
    } = draw_ports(ui, &frame, live);
    if wire_started.is_some() {
        app.canvas.wire = wire_started;
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
    let (problems, added) = apply_edits(
        app,
        &lib,
        Edits {
            moves,
            drag_end: drag_end.clone(),
            landing,
            wire_edit,
            wire_end,
            wire_break,
            connect,
            drop,
        },
    );
    if drag_end.is_some() {
        app.canvas.moving = None;
    }
    if let Some(sheet) = toggle_sheet {
        if !app.canvas.collapsed.remove(&sheet) {
            app.canvas.collapsed.insert(sheet);
        }
    }
    if let Some((a, b)) = sheet_swap {
        swap_sheets(app, ui, &lib, a, b, &bands, &xf, rect);
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
