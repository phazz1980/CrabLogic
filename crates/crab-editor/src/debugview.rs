//! Online debugging in the editor: live values on the sheets, forcing,
//! setpoints, a window with inputs and variables and a simple chart window.

use std::time::Instant;

use crab_codegen::Program;
use crab_core::{Layout, Library, ParamKind, PortRef, Project, SignalType};
use crab_debug::{Command, DebugTable, Live, MasterSession, ModuleMap, Source, Value};
use eframe::egui::{self, Color32, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::theme::Palette;

pub struct DebugState {
    session: MasterSession,
    pub table: DebugTable,
    pub port: String,
    /// Snapshot taken at the start of each frame.
    pub live: Live,
    /// Entries shown in the chart window.
    pub chart: Vec<usize>,
    /// The cycle time window is open.
    pub diagnostics: bool,
    /// The window with inputs and variables is open.
    pub values: bool,
    /// Outputs of input tags (blocks with a pin that produce values), in schema order.
    pub inputs: Vec<(PortRef, usize)>,
    /// Cycle time reports of the last two minutes, for the diagnostics charts.
    stat_history: Vec<StatSample>,
    started: Instant,
    /// Does the project in the editor match the board's? Recomputed when the board's
    /// hash or the schema changes.
    pub same_project: Option<bool>,
    checked: Option<([u8; 8], u64)>,
    /// The question "download the board's version?" was already put this session.
    pub hash_asked: bool,
}

/// One cycle time report with the time it arrived (seconds since the session start).
struct StatSample {
    t: f64,
    stats: crab_debug::Stats,
}

/// How far behind the board the wires of 0/1 signals are, seconds: a bit more than a poll.
const SHOW_DELAY: f64 = 0.35;

/// How long the diagnostics charts keep their samples, seconds.
const STAT_HISTORY_SECS: f64 = 120.0;

impl DebugState {
    pub fn start(
        project: &Project,
        lib: &Library,
        port: &str,
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        let project = &project.without_disabled();
        let program = Program::build(project, lib).map_err(|e| e.to_string())?;
        let table = crab_codegen::debug::debug_table(&program);
        let inputs = input_ports(project, lib, &table);
        let mut modules = Vec::new();
        for sheet in program.sheets() {
            let built = program
                .wasm_module_with(sheet, true)
                .map_err(|e| crate::i18n::fmt("лист {0}: {1}", &[&sheet, &e]))?;
            let slots = built
                .watched
                .iter()
                .map(|label| {
                    let port = label.parse::<PortRef>().ok()?;
                    table.output_index(&port)
                })
                .collect();
            modules.push(ModuleMap {
                crc: crab_build::mcu_module::crc32(&built.wasm),
                sheet,
                name: built.name,
                slots,
            });
        }
        let ctx = ctx.clone();
        let session = MasterSession::start(
            table.clone(),
            modules,
            crab_debug::transport::serial(port),
            move || ctx.request_repaint(),
        );
        Ok(Self {
            live: session.snapshot(),
            session,
            table,
            port: port.to_owned(),
            chart: Vec::new(),
            diagnostics: false,
            values: false,
            inputs,
            stat_history: Vec::new(),
            started: Instant::now(),
            same_project: None,
            checked: None,
            hash_asked: false,
        })
    }

    pub fn refresh(&mut self) {
        self.live = self.session.snapshot();
        self.record_stats();
    }

    /// Keeps each new cycle time report for the diagnostics charts.
    fn record_stats(&mut self) {
        let Some(stats) = self.live.stats.clone().filter(|_| self.online()) else {
            return;
        };
        if self.stat_history.last().is_some_and(|l| l.stats == stats) {
            return;
        }
        let t = self.started.elapsed().as_secs_f64();
        self.stat_history.push(StatSample { t, stats });
        self.stat_history.retain(|s| s.t >= t - STAT_HISTORY_SECS);
    }

    /// Points of one figure of the stored reports.
    fn series(&self, value: impl Fn(&crab_debug::Stats) -> Option<f64>) -> Vec<(f64, f64)> {
        self.stat_history
            .iter()
            .filter_map(|s| value(&s.stats).map(|v| (s.t, v)))
            .collect()
    }

    /// Compares the editor's project with the one the board was built from.
    pub fn check_project(&mut self, project: &Project, revision: u64) {
        let Some(on_board) = self.live.project_hash else {
            self.same_project = None;
            self.checked = None;
            return;
        };
        if self.checked == Some((on_board, revision)) {
            return;
        }
        self.checked = Some((on_board, revision));
        self.same_project = project
            .without_disabled()
            .content_hash()
            .ok()
            .map(|mine| mine == on_board);
    }

    pub fn stop(self) {
        self.session.stop();
    }

    /// Values are meaningful only when the board runs firmware for this schema.
    pub fn online(&self) -> bool {
        self.live.connected && self.live.schema_matches == Some(true)
    }

    /// Live value of a block output: (value, forced).
    pub fn output(&self, port: &PortRef) -> Option<(Value, bool)> {
        if !self.online() {
            return None;
        }
        let i = self.table.output_index(port)?;
        Some((self.shown(i)?, self.live.forced[i].is_some()))
    }

    /// Value of table entry `i` for the schema. A 0/1 signal the board reports with the time of
    /// every change is shown as it was `SHOW_DELAY` ago, read from its history: wires blink in
    /// step with the board instead of following the polls.
    fn shown(&self, i: usize) -> Option<Value> {
        let v = self.live.values[i]?;
        if let (Value::Bool(_), true, Some(clock)) = (v, self.live.events, self.live.clock) {
            let target = clock.elapsed().as_secs_f64() - SHOW_DELAY;
            let h = &self.live.history[i];
            let k = h.partition_point(|p| p.0 <= target);
            if k > 0 {
                return Some(Value::Bool(h[k - 1].1 != 0.0));
            }
        }
        Some(v)
    }

    /// The schema must be redrawn often while it shows signals that follow the history.
    pub fn animating(&self) -> bool {
        self.online() && self.live.events
    }

    pub fn var(&self, name: &str) -> Option<(Value, bool)> {
        if !self.online() {
            return None;
        }
        let i = self.table.var_index(name)?;
        Some((self.shown(i)?, self.live.forced[i].is_some()))
    }

    /// Live string on a block output (not forceable).
    pub fn text_output(&self, port: &PortRef) -> Option<&str> {
        if !self.online() {
            return None;
        }
        let i = self.table.text_output_index(port)?;
        self.live.texts[i].as_deref()
    }

    /// IP address the board reports on the `address` output of a Wi-Fi block, if any.
    pub fn device_address(&self, blocks: &[crab_core::Block]) -> Option<String> {
        // The master firmware reports its address itself.
        if let Some(ip) = self.live.address.as_deref().filter(|a| !a.is_empty()) {
            return self.online().then(|| ip.to_owned());
        }
        self.table.texts.iter().enumerate().find_map(|(i, t)| {
            let Source::Output(p) = &t.source else {
                return None;
            };
            let is_wifi =
                p.port == "address" && blocks.iter().any(|b| b.id == p.block && b.kind == "wifi");
            let ip = self.live.texts.get(i)?.as_deref()?.trim();
            (self.online() && is_wifi && !ip.is_empty()).then(|| ip.to_owned())
        })
    }

    pub fn text_var(&self, name: &str) -> Option<&str> {
        if !self.online() {
            return None;
        }
        let i = self.table.text_var_index(name)?;
        self.live.texts[i].as_deref()
    }

    pub fn send(&self, command: Command) {
        self.session.send(command);
    }

    pub fn name(&self, index: usize) -> String {
        match &self.table.entries[index].source {
            Source::Output(p) => p.to_string(),
            Source::Var(v) => v.clone(),
        }
    }
}

/// Outputs of blocks with a pin parameter: hardware inputs and sensors.
fn input_ports(project: &Project, lib: &Library, table: &DebugTable) -> Vec<(PortRef, usize)> {
    let logic = &project.logic;
    let mut out = Vec::new();
    for b in &logic.blocks {
        let Some(def) = lib.resolve(logic, b).map(|l| &l.def) else {
            continue;
        };
        if !def.params.iter().any(|p| p.kind == ParamKind::Pin) {
            continue;
        }
        for port in def.active_outputs(&b.params) {
            let port = PortRef::new(&b.id, &port.name);
            if let Some(i) = table.output_index(&port) {
                out.push((port, i));
            }
        }
    }
    out
}

fn as_f64(v: Value) -> f64 {
    match v {
        Value::Bool(b) => f64::from(u8::from(b)),
        Value::Int(i) => f64::from(i),
        Value::Float(f) => f64::from(f),
    }
}

/// Number field for a new value of entry `index`; remembered between frames,
/// starts from the forced (or current) value.
fn number_edit(ui: &mut Ui, dbg: &DebugState, index: usize, salt: &str) -> Value {
    let ty = dbg.table.entries[index].ty;
    let id = Id::unique(("debug_new_value", salt, index));
    let current = dbg.live.forced[index]
        .or(dbg.live.values[index])
        .map_or(0.0, as_f64);
    let mut v: f64 = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || current));
    let speed = if ty == SignalType::Int { 1.0 } else { 0.1 };
    ui.add(egui::DragValue::new(&mut v).speed(speed))
        .on_hover_text(crate::i18n::tr(
            "Перетащите или щёлкните дважды, чтобы ввести число",
        ));
    ui.data_mut(|d| d.insert_temp(id, v));
    if ty == SignalType::Int {
        Value::Int(v.round() as i32)
    } else {
        Value::Float(v as f32)
    }
}

/// Context menu items for one observed value (block output or variable).
pub fn value_menu(ui: &mut Ui, dbg: &mut DebugState, index: usize) {
    let entry = &dbg.table.entries[index];
    let ty = entry.ty;
    let is_var = matches!(entry.source, Source::Var(_));
    let forced = dbg.live.forced[index];
    ui.label(egui::RichText::new(dbg.name(index)).strong());
    match ty {
        SignalType::Bool => {
            for b in [true, false] {
                if ui
                    .button(crate::i18n::fmt("Форсировать {0}", &[&u8::from(b)]))
                    .clicked()
                {
                    dbg.send(Command::Force {
                        index,
                        value: Some(Value::Bool(b)),
                    });
                }
            }
            if is_var {
                for b in [true, false] {
                    if ui
                        .button(crate::i18n::fmt("Записать {0}", &[&u8::from(b)]))
                        .on_hover_text(crate::i18n::tr(
                            "Значение остаётся, пока его не перезапишет схема",
                        ))
                        .clicked()
                    {
                        dbg.send(Command::Set {
                            index,
                            value: Value::Bool(b),
                        });
                    }
                }
            }
        }
        SignalType::Int | SignalType::Float => {
            let value = ui
                .horizontal(|ui| {
                    ui.label(crate::i18n::tr("Значение"));
                    number_edit(ui, dbg, index, "menu")
                })
                .inner;
            if ui.button(crate::i18n::tr("Форсировать")).clicked() {
                dbg.send(Command::Force {
                    index,
                    value: Some(value),
                });
            }
            if is_var
                && ui
                    .button(crate::i18n::tr("Записать один раз"))
                    .on_hover_text(crate::i18n::tr(
                        "Уставка: значение остаётся, пока его не перезапишет схема",
                    ))
                    .clicked()
            {
                dbg.send(Command::Set { index, value });
            }
        }
        // Not in the debug table.
        SignalType::String | SignalType::Any | SignalType::Number | SignalType::Same => {}
    }
    if forced.is_some() && ui.button(crate::i18n::tr("Снять форсировку")).clicked() {
        dbg.send(Command::Force { index, value: None });
    }
    ui.separator();
    let on_chart = dbg.chart.contains(&index);
    if !on_chart && ui.button(crate::i18n::tr("Добавить на график")).clicked() {
        dbg.chart.push(index);
    }
    if on_chart && ui.button(crate::i18n::tr("Убрать с графика")).clicked() {
        dbg.chart.retain(|&i| i != index);
    }
}

/// Window with all inputs and variables: live values, writing and forcing.
pub fn values_window(dbg: &mut DebugState, pal: &Palette, ctx: &egui::Context) {
    if !dbg.values {
        return;
    }
    let mut open = true;
    egui::Window::new(crate::i18n::tr("Входы и переменные"))
        .open(&mut open)
        .default_width(440.0)
        .show(ctx, |ui| {
            if !dbg.online() {
                ui.label(crate::i18n::tr(
                    "Нет данных: плата не на связи или прошивка для другой схемы",
                ));
                return;
            }
            let texts: Vec<usize> = (0..dbg.table.texts.len())
                .filter(|&i| matches!(dbg.table.texts[i].source, Source::Var(_)))
                .collect();
            let vars: Vec<usize> = (0..dbg.table.entries.len())
                .filter(|&i| matches!(dbg.table.entries[i].source, Source::Var(_)))
                .collect();
            let inputs: Vec<usize> = dbg.inputs.iter().map(|&(_, i)| i).collect();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (title, rows) in [
                    (crate::i18n::tr("Входы"), inputs),
                    (crate::i18n::tr("Переменные"), vars),
                ] {
                    ui.strong(title);
                    if rows.is_empty() {
                        ui.weak(crate::i18n::tr("нет"));
                    }
                    egui::Grid::new(("debug_values", title))
                        .num_columns(3)
                        .striped(true)
                        .show(ui, |ui| {
                            for index in rows {
                                value_row(ui, dbg, pal, index);
                                ui.end_row();
                            }
                        });
                    ui.add_space(6.0);
                }
                if !texts.is_empty() {
                    ui.strong(crate::i18n::tr("Строки"));
                    egui::Grid::new("debug_texts")
                        .num_columns(2)
                        .striped(true)
                        .show(ui, |ui| {
                            for i in texts {
                                let Source::Var(name) = &dbg.table.texts[i].source else {
                                    continue;
                                };
                                ui.label(name);
                                let value = dbg.live.texts[i].as_deref().map_or("—".into(), quoted);
                                ui.monospace(value);
                                ui.end_row();
                            }
                        });
                }
            });
            ui.separator();
            ui.weak(crate::i18n::tr(
                "«Записать» меняет переменную один раз — дальше её может перезаписать схема. \
                 «Форс.» держит значение, пока форсировку не снимут; форсированный вход \
                 не зависит от сигнала на выводе платы.",
            ));
        });
    if !open {
        dbg.values = false;
    }
}

/// Name, live value and controls of one entry, as three grid cells.
fn value_row(ui: &mut Ui, dbg: &mut DebugState, pal: &Palette, index: usize) {
    let entry = &dbg.table.entries[index];
    let ty = entry.ty;
    let is_var = matches!(entry.source, Source::Var(_));
    let forced = dbg.live.forced[index];
    let current = dbg.live.values[index];
    ui.label(dbg.name(index))
        .context_menu(|ui| value_menu(ui, dbg, index));
    let text = current.map_or("—".to_owned(), |v| v.to_string());
    if forced.is_some() {
        ui.label(
            egui::RichText::new(format!("{text} F"))
                .monospace()
                .color(pal.warning),
        )
        .on_hover_text(crate::i18n::tr("Значение форсировано"));
    } else {
        ui.monospace(text);
    }
    ui.horizontal(|ui| {
        match ty {
            SignalType::Bool => {
                let on = current == Some(Value::Bool(true));
                if is_var
                    && ui
                        .button(if on {
                            crate::i18n::tr("Записать 0")
                        } else {
                            crate::i18n::tr("Записать 1")
                        })
                        .on_hover_text(crate::i18n::tr("Переключить переменную один раз"))
                        .clicked()
                {
                    dbg.send(Command::Set {
                        index,
                        value: Value::Bool(!on),
                    });
                }
                for b in [true, false] {
                    let selected = forced == Some(Value::Bool(b));
                    if ui
                        .selectable_label(selected, crate::i18n::fmt("Форс. {0}", &[&u8::from(b)]))
                        .clicked()
                    {
                        dbg.send(Command::Force {
                            index,
                            value: Some(Value::Bool(b)),
                        });
                    }
                }
            }
            SignalType::Int | SignalType::Float => {
                let value = number_edit(ui, dbg, index, "window");
                if is_var && ui.button(crate::i18n::tr("Записать")).clicked() {
                    dbg.send(Command::Set { index, value });
                }
                if ui.button(crate::i18n::tr("Форс.")).clicked() {
                    dbg.send(Command::Force {
                        index,
                        value: Some(value),
                    });
                }
            }
            SignalType::String | SignalType::Any | SignalType::Number | SignalType::Same => {}
        }
        if forced.is_some()
            && ui
                .small_button(crate::i18n::tr("Снять"))
                .on_hover_text(crate::i18n::tr("Снять форсировку"))
                .clicked()
        {
            dbg.send(Command::Force { index, value: None });
        }
    });
}

/// Where a value label goes relative to its anchor point.
#[derive(Clone, Copy)]
pub enum Place {
    /// Above and to the right of a port.
    Right,
    /// Above and to the left of a port (the input end of a wire).
    Left,
}

/// Small label with a live value next to a port.
pub fn value_label(
    painter: &egui::Painter,
    pal: &Palette,
    anchor: Pos2,
    value: Value,
    forced: bool,
    zoom: f32,
    place: Place,
) -> Rect {
    let text = if forced {
        format!("{value} F")
    } else {
        value.to_string()
    };
    label(painter, pal, anchor, text, forced, zoom, place)
}

/// Live string next to a port, in quotes and cut to a readable length.
pub fn text_label(
    painter: &egui::Painter,
    pal: &Palette,
    anchor: Pos2,
    text: &str,
    zoom: f32,
    place: Place,
) -> Rect {
    label(painter, pal, anchor, quoted(text), false, zoom, place)
}

/// `"text"` with control characters made visible, at most 24 characters.
pub fn quoted(text: &str) -> String {
    const MAX: usize = 24;
    let shown: String = text
        .chars()
        .take(MAX)
        .map(|c| if c.is_control() { '·' } else { c })
        .collect();
    let more = if text.chars().count() > MAX {
        "…"
    } else {
        ""
    };
    format!("\"{shown}{more}\"")
}

fn label(
    painter: &egui::Painter,
    pal: &Palette,
    anchor: Pos2,
    text: String,
    forced: bool,
    zoom: f32,
    place: Place,
) -> Rect {
    let font = egui::FontId::monospace(9.5 * zoom.clamp(0.6, 1.6));
    let galley = painter.layout_no_wrap(text, font, if forced { Color32::WHITE } else { pal.text });
    let size = galley.size() + Vec2::new(6.0, 2.0);
    let min = match place {
        Place::Right => anchor + Vec2::new(4.0, -size.y - 1.0),
        Place::Left => anchor + Vec2::new(-size.x - 4.0, -size.y - 1.0),
    };
    let rect = Rect::from_min_size(min, size);
    let fill = if forced {
        pal.warning
    } else {
        pal.sheet_header
    };
    painter.rect(
        rect,
        3.0,
        fill,
        Stroke::new(1.0, pal.block_border),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.min + Vec2::new(3.0, 1.0), galley, pal.text);
    rect
}

/// Window with a strip chart per selected value (last minute).
pub fn chart_window(dbg: &mut DebugState, pal: &Palette, ctx: &egui::Context) {
    if dbg.chart.is_empty() {
        return;
    }
    let mut open = true;
    let mut remove = None;
    egui::Window::new(crate::i18n::tr("Графики"))
        .open(&mut open)
        .default_size([460.0, 120.0 * dbg.chart.len() as f32])
        .show(ctx, |ui| {
            for &index in &dbg.chart.clone() {
                ui.horizontal(|ui| {
                    ui.strong(dbg.name(index));
                    if let Some(v) = dbg.live.values[index] {
                        ui.monospace(v.to_string());
                    }
                    if ui.small_button("×").clicked() {
                        remove = Some(index);
                    }
                });
                strip(ui, pal, index, &dbg.live.history[index]);
            }
        });
    if !open {
        dbg.chart.clear();
    }
    if let Some(i) = remove {
        dbg.chart.retain(|&c| c != i);
    }
}

/// Window with cycle time of the whole program and of each sheet.
pub fn diagnostics_window(
    dbg: &mut DebugState,
    pal: &Palette,
    layout: Option<&Layout>,
    ctx: &egui::Context,
) {
    if !dbg.diagnostics {
        return;
    }
    let mut open = true;
    egui::Window::new(crate::i18n::tr("Диагностика"))
        .open(&mut open)
        .default_width(360.0)
        .show(ctx, |ui| {
            let Some(s) = dbg.live.stats.clone().filter(|_| dbg.online()) else {
                ui.label(crate::i18n::tr(
                    "Нет данных: плата не на связи или прошивка без диагностики",
                ));
                return;
            };
            let us = |v: u32| crate::i18n::fmt("{0} мкс", &[&v]);
            egui::Grid::new("diag_total")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label(crate::i18n::tr("Цикл, последний"));
                    ui.monospace(us(s.last_us));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Цикл, средний"));
                    ui.monospace(us(s.avg_us));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Цикл, максимальный"));
                    ui.strong(us(s.max_us));
                    ui.end_row();
                    if let Some(bg) = &s.background {
                        ui.label(crate::i18n::tr("Ядро 0, проход последний"))
                            .on_hover_text(crate::i18n::tr(
                                "Фоновая задача: сеть и платы с признаком «фоновая»",
                            ));
                        ui.monospace(us(bg.last_us));
                        ui.end_row();
                        ui.label(crate::i18n::tr("Ядро 0, проход максимальный"));
                        ui.strong(us(bg.max_us));
                        ui.end_row();
                    }
                    ui.label(crate::i18n::tr("Период loop(), макс"))
                        .on_hover_text(crate::i18n::tr(
                            "Время между запусками loop(): схема, отладка и система",
                        ));
                    ui.monospace(us(s.period_max_us));
                    ui.end_row();
                    ui.label(crate::i18n::tr("Циклов за 0,5 с"));
                    ui.monospace(s.cycles.to_string());
                    ui.end_row();
                    if s.free_heap > 0 {
                        ui.label(crate::i18n::tr("Свободная память"));
                        ui.monospace(crate::i18n::fmt("{0} КБ", &[&(s.free_heap / 1024)]));
                        ui.end_row();
                    }
                });
            if !s.sheets.is_empty() {
                ui.separator();
                egui::Grid::new("diag_sheets")
                    .num_columns(3)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong(crate::i18n::tr("Лист"));
                        ui.strong(crate::i18n::tr("Последний"));
                        ui.strong(crate::i18n::tr("Макс"));
                        ui.end_row();
                        for sh in &s.sheets {
                            let name = layout
                                .and_then(|l| l.sheet(sh.sheet))
                                .filter(|i| !i.name.is_empty())
                                .map_or_else(
                                    || sh.sheet.to_string(),
                                    |i| format!("{} {}", sh.sheet, i.name),
                                );
                            ui.label(name);
                            ui.monospace(us(sh.last_us));
                            ui.monospace(us(sh.max_us));
                            ui.end_row();
                        }
                    });
            }
            ui.separator();
            let charts = [
                (
                    crate::i18n::tr("Цикл, средний, мкс"),
                    dbg.series(|s| Some(f64::from(s.avg_us))),
                ),
                (
                    crate::i18n::tr("Цикл, максимальный, мкс"),
                    dbg.series(|s| Some(f64::from(s.max_us))),
                ),
                (
                    crate::i18n::tr("Ядро 0, проход, мкс"),
                    dbg.series(|s| s.background.as_ref().map(|b| f64::from(b.last_us))),
                ),
                (
                    crate::i18n::tr("Свободная память, КБ"),
                    dbg.series(|s| (s.free_heap > 0).then(|| f64::from(s.free_heap) / 1024.0)),
                ),
            ];
            for (title, points) in charts.iter().filter(|(_, p)| !p.is_empty()) {
                ui.horizontal(|ui| {
                    ui.strong(*title);
                    if let Some(&(_, v)) = points.last() {
                        ui.monospace(format!("{v:.0}"));
                    }
                });
                strip(ui, pal, title.to_string(), points);
            }
            ui.separator();
            if ui
                .button(crate::i18n::tr("Сбросить максимумы"))
                .on_hover_text(crate::i18n::tr(
                    "Начать фиксацию максимального времени заново",
                ))
                .clicked()
            {
                dbg.send(Command::ResetStats);
            }
        });
    if !open {
        dbg.diagnostics = false;
    }
}

/// What one chart shows: the width of the window and how far its right edge is behind the
/// newest point (zero follows the data).
#[derive(Clone, Copy)]
struct ChartView {
    span: f64,
    back: f64,
}

impl Default for ChartView {
    fn default() -> Self {
        Self {
            span: DEFAULT_SPAN,
            back: 0.0,
        }
    }
}

const DEFAULT_SPAN: f64 = 60.0;
const MIN_SPAN: f64 = 0.5;
const MAX_SPAN: f64 = 3600.0;
/// Heights of the plot and of the strip with the time labels.
const PLOT_H: f32 = 80.0;
const AXIS_H: f32 = 13.0;

/// A step of the time grid (seconds) that gives about one label per `want` seconds.
fn time_step(want: f64) -> f64 {
    const STEPS: [f64; 18] = [
        0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0,
        1800.0, 3600.0, 7200.0,
    ];
    STEPS
        .iter()
        .copied()
        .find(|s| *s >= want)
        .unwrap_or(STEPS[STEPS.len() - 1])
}

/// Time since the start of the session: `m:ss`, `h:mm:ss`, with tenths when `fine`.
fn time_label(t: f64, fine: bool) -> String {
    let t = t.max(0.0);
    let whole = t as u64;
    let (h, m, sec) = (whole / 3600, whole / 60 % 60, whole % 60);
    let frac = if fine {
        format!(".{}", ((t - whole as f64) * 10.0) as u64)
    } else {
        String::new()
    };
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}{frac}")
    } else {
        format!("{m}:{sec:02}{frac}")
    }
}

/// The value a chart has at time `t`: the last point not after it.
fn value_at(points: &[(f64, f64)], t: f64) -> Option<f64> {
    let k = points.partition_point(|p| p.0 <= t);
    k.checked_sub(1).map(|k| points[k].1)
}

/// A chart of `points` (time since the start of the session, value) with a time axis. The
/// wheel zooms around the pointer, dragging moves the window, a double click goes back to the
/// last minute that follows the data.
fn strip(
    ui: &mut Ui,
    pal: &Palette,
    key: impl std::hash::Hash + std::fmt::Debug,
    points: &[(f64, f64)],
) {
    let id = ui.scope_id().with(("chart", key));
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), PLOT_H + AXIS_H),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, pal.sheet);
    let plot = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.max.y - AXIS_H));
    let Some(&(t_last, _)) = points.last() else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            crate::i18n::tr("нет данных"),
            egui::FontId::proportional(11.0),
            pal.weak,
        );
        return;
    };
    let mut view: ChartView = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let width = f64::from(plot.width().max(1.0));
    if response.double_clicked() {
        view = ChartView::default();
    }
    if response.hovered() {
        // The wheel belongs to the chart here, not to the window around it.
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        if wheel != 0.0 {
            ui.input_mut(|i| {
                i.smooth_scroll_delta = Vec2::ZERO;
            });
            let x = response
                .hover_pos()
                .map_or(0.5, |p| f64::from((p.x - plot.min.x) / plot.width()));
            let end = t_last - view.back;
            let under = end - view.span * (1.0 - x);
            let span = (view.span * (-f64::from(wheel) * 0.004).exp()).clamp(MIN_SPAN, MAX_SPAN);
            // The moment under the pointer stays under it.
            view.back = (t_last - (under + span * (1.0 - x))).max(0.0);
            view.span = span;
        }
    }
    if response.dragged() {
        let dt = f64::from(response.drag_delta().x) / width * view.span;
        view.back = (view.back + dt).max(0.0);
    }
    let first = points.first().map_or(0.0, |p| p.0);
    view.back = view.back.min((t_last - first).max(0.0));
    ui.data_mut(|d| d.insert_temp(id, view));

    let t_end = t_last - view.back;
    let t_start = t_end - view.span;
    // The points of the window and one on each side, so the line reaches the edges.
    let a = points.partition_point(|p| p.0 < t_start).saturating_sub(1);
    let b = (points.partition_point(|p| p.0 <= t_end) + 1).min(points.len());
    let visible = &points[a..b];
    let (lo, hi) = visible.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
        (lo.min(p.1), hi.max(p.1))
    });
    let span_y = if hi - lo < 1e-9 { 1.0 } else { hi - lo };
    let to_x = |t: f64| plot.min.x + ((t - t_start) / view.span) as f32 * plot.width();
    let to_screen = |t: f64, y: f64| {
        Pos2::new(
            to_x(t),
            plot.max.y - 6.0 - ((y - lo) / span_y) as f32 * (plot.height() - 12.0),
        )
    };
    let font = egui::FontId::monospace(9.0);
    // Time grid and labels.
    let step = time_step(view.span * 70.0 / width);
    let grid = pal.weak.gamma_multiply(0.25);
    let mut t = (t_start / step).ceil() * step;
    while t <= t_end {
        let x = to_x(t);
        painter.line_segment(
            [Pos2::new(x, plot.min.y), Pos2::new(x, plot.max.y)],
            Stroke::new(1.0, grid),
        );
        painter.text(
            Pos2::new(x + 2.0, rect.max.y - 1.0),
            egui::Align2::LEFT_BOTTOM,
            time_label(t, step < 1.0),
            font.clone(),
            pal.weak,
        );
        t += step;
    }
    let line: Vec<Pos2> = visible.iter().map(|p| to_screen(p.0, p.1)).collect();
    painter
        .with_clip_rect(plot)
        .line(line, Stroke::new(1.5, pal.accent));
    painter.text(
        plot.left_top() + Vec2::new(3.0, 2.0),
        egui::Align2::LEFT_TOP,
        format!("{hi:.2}"),
        font.clone(),
        pal.weak,
    );
    painter.text(
        plot.left_bottom() + Vec2::new(3.0, -2.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{lo:.2}"),
        font.clone(),
        pal.weak,
    );
    if view.back > 0.0 {
        painter.text(
            plot.right_top() + Vec2::new(-3.0, 2.0),
            egui::Align2::RIGHT_TOP,
            crate::i18n::fmt("−{0} с от конца", &[&format!("{:.1}", view.back)]),
            font.clone(),
            pal.weak,
        );
    }
    // Under the pointer: the time and the value there.
    if let Some(pos) = response.hover_pos().filter(|p| plot.contains(*p)) {
        let t = t_start + f64::from((pos.x - plot.min.x) / plot.width()) * view.span;
        painter.line_segment(
            [Pos2::new(pos.x, plot.min.y), Pos2::new(pos.x, plot.max.y)],
            Stroke::new(1.0, pal.weak),
        );
        if let Some(v) = value_at(points, t) {
            let right = pos.x > plot.center().x;
            let (align, dx) = if right {
                (egui::Align2::RIGHT_TOP, -4.0)
            } else {
                (egui::Align2::LEFT_TOP, 4.0)
            };
            painter.text(
                Pos2::new(pos.x + dx, plot.min.y + 12.0),
                align,
                format!("{}  {v:.3}", time_label(t, true)),
                font,
                pal.text,
            );
        }
    }
}

#[cfg(test)]
mod chart_tests {
    use super::*;

    #[test]
    fn time_grid_step_grows_with_the_window() {
        assert_eq!(time_step(0.03), 0.05);
        assert_eq!(time_step(7.0), 10.0);
        assert_eq!(time_step(70.0 * 60.0 / 400.0), 15.0);
        assert_eq!(time_step(1e9), 7200.0);
    }

    #[test]
    fn time_labels() {
        assert_eq!(time_label(75.0, false), "1:15");
        assert_eq!(time_label(3725.0, false), "1:02:05");
        assert_eq!(time_label(1.25, true), "0:01.2");
        assert_eq!(time_label(-3.0, false), "0:00");
    }

    #[test]
    fn value_under_the_pointer_is_the_last_point_before_it() {
        let p = [(1.0, 0.0), (2.0, 1.0), (2.0, 0.0), (5.0, 1.0)];
        assert_eq!(value_at(&p, 0.5), None);
        assert_eq!(value_at(&p, 1.5), Some(0.0));
        assert_eq!(
            value_at(&p, 2.0),
            Some(0.0),
            "an upright edge ends on the new level"
        );
        assert_eq!(value_at(&p, 9.0), Some(1.0));
    }
}
