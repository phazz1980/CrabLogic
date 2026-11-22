//! The tables of the documentation: the terminals of the board, the wires and the bill of
//! materials. Each is a [`Table`]: it is drawn on sheets (as many as the rows need) and written
//! as CSV for a spreadsheet.

use crab_core::{DeviceKind, Project};

use crate::canvas::{Align, Canvas, CHAR_W, INK, TEXT as FONT, TEXT_L, THICK, THIN, WEAK};
use crate::doc::{Page, Sheet};
use crate::lang::Lang;
use crate::nets::{Nets, WireClass};

const ROW_H: f32 = 20.0;
const HEAD_H: f32 = 30.0;
/// Room above the table for its caption.
const CAPTION_ROOM: f32 = 50.0;
/// Room below the last row for the total.
const FOOTER_ROOM: f32 = 40.0;

/// A table: the head, the rows and the total line.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// Caption of the table and of its sheets.
    pub title: String,
    pub head: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Relative widths of the columns.
    pub widths: Vec<f32>,
    /// A line under the table (the total); drawn on the last sheet, not part of the CSV.
    pub footer: Option<String>,
}

/// The three tables of a panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Tables {
    pub terminals: Table,
    pub wires: Table,
    pub bom: Table,
}

/// Name of a kind of device in the bill of materials and in the editor.
pub fn device_kind_name(kind: DeviceKind, lang: Lang) -> &'static str {
    match kind {
        DeviceKind::Feed => lang.pick("Ввод питания", "Supply feed"),
        DeviceKind::Breaker => lang.pick("Автоматический выключатель", "Circuit breaker"),
        DeviceKind::Rcd => lang.pick("УЗО", "Residual-current device"),
        DeviceKind::Fuse => lang.pick("Предохранитель", "Fuse"),
        DeviceKind::Switch => lang.pick("Рубильник", "Disconnector"),
        DeviceKind::Psu => lang.pick("Блок питания", "Power supply"),
        DeviceKind::Contactor => lang.pick("Контактор", "Contactor"),
        DeviceKind::Relay => lang.pick("Реле", "Relay"),
        DeviceKind::Motor => lang.pick("Двигатель", "Motor"),
        DeviceKind::Load => lang.pick("Нагрузка", "Load"),
        DeviceKind::Terminal => lang.pick("Клеммник", "Terminal block"),
        DeviceKind::Sensor => lang.pick("Датчик", "Sensor"),
        DeviceKind::Button => lang.pick("Кнопка", "Push button"),
        DeviceKind::Aux => lang.pick("Дополнительный контакт", "Auxiliary contact"),
    }
}

fn class_name(class: WireClass, lang: Lang) -> &'static str {
    match class {
        WireClass::Power => lang.pick("Силовой", "Power"),
        WireClass::Control => lang.pick("Управление 24 В", "Control 24 V"),
        WireClass::Protective => lang.pick("Защитный PE", "Protective earth"),
        WireClass::Signal => lang.pick("Сигнал", "Signal"),
    }
}

/// The three tables of the panel of `project`.
pub fn build_tables(project: &Project, nets: &Nets, lang: Lang) -> Tables {
    Tables {
        terminals: terminals_table(project, nets, lang),
        wires: wires_table(nets, lang),
        bom: bom_table(project, lang),
    }
}

/// Terminals of the board: the terminal, the input or output of the project, the device and the
/// wire.
pub fn terminals_table(project: &Project, nets: &Nets, lang: Lang) -> Table {
    let rows = nets
        .board_terminals
        .iter()
        .map(|t| {
            let device = project.panel.device(&t.device);
            let direction = match device.map(|d| d.kind) {
                Some(DeviceKind::Contactor | DeviceKind::Relay) => lang.pick("выход", "output"),
                _ => lang.pick("вход", "input"),
            };
            vec![
                format!("{}:{}", crate::nets::BOARD, t.terminal),
                t.tag.clone(),
                direction.to_owned(),
                device.map_or_else(|| t.device.clone(), |d| d.designation()),
                t.wire.to_string(),
            ]
        })
        .collect::<Vec<_>>();
    Table {
        title: lang
            .pick("Перечень клемм контроллера", "Controller terminals")
            .to_owned(),
        head: [
            lang.pick("Клемма", "Terminal"),
            lang.pick("Вход или выход проекта", "Project input or output"),
            lang.pick("Направление", "Direction"),
            lang.pick("Устройство", "Device"),
            lang.pick("Провод №", "Wire no."),
        ]
        .map(str::to_owned)
        .to_vec(),
        widths: vec![2.0, 3.0, 2.0, 2.0, 1.5],
        footer: Some(format!(
            "{} {}",
            lang.pick("Клемм:", "Terminals:"),
            rows.len()
        )),
        rows,
    }
}

/// All the wires, by number.
pub fn wires_table(nets: &Nets, lang: Lang) -> Table {
    let mut wires: Vec<_> = nets.wires.iter().collect();
    wires.sort_by_key(|w| w.number);
    let rows = wires
        .iter()
        .map(|w| {
            vec![
                w.number.to_string(),
                class_name(w.class, lang).to_owned(),
                w.from.to_string(),
                w.to.to_string(),
            ]
        })
        .collect::<Vec<_>>();
    Table {
        title: lang.pick("Перечень проводов", "Wire list").to_owned(),
        head: [
            lang.pick("Провод №", "Wire no."),
            lang.pick("Тип", "Type"),
            lang.pick("Откуда", "From"),
            lang.pick("Куда", "To"),
        ]
        .map(str::to_owned)
        .to_vec(),
        widths: vec![1.5, 3.0, 3.0, 3.0],
        footer: Some(format!(
            "{} {}",
            lang.pick("Проводов:", "Wires:"),
            rows.len()
        )),
        rows,
    }
}

/// The bill of materials: devices of one kind, rating and part number are one line.
pub fn bom_table(project: &Project, lang: Lang) -> Table {
    struct Line<'a> {
        kind: DeviceKind,
        rating: &'a str,
        part: &'a str,
        designations: Vec<String>,
    }
    let mut lines: Vec<Line> = Vec::new();
    for d in &project.panel.devices {
        match lines
            .iter_mut()
            .find(|l| l.kind == d.kind && l.rating == d.rating && l.part == d.part)
        {
            Some(line) => line.designations.push(d.designation()),
            None => lines.push(Line {
                kind: d.kind,
                rating: &d.rating,
                part: &d.part,
                designations: vec![d.designation()],
            }),
        }
    }
    let total: usize = lines.iter().map(|l| l.designations.len()).sum();
    let rows = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            vec![
                (i + 1).to_string(),
                l.designations.join(", "),
                device_kind_name(l.kind, lang).to_owned(),
                l.rating.to_owned(),
                l.part.to_owned(),
                l.designations.len().to_string(),
            ]
        })
        .collect();
    Table {
        title: lang.pick("Спецификация", "Bill of materials").to_owned(),
        head: [
            lang.pick("Поз.", "Item"),
            lang.pick("Обозначение", "Designation"),
            lang.pick("Наименование", "Name"),
            lang.pick("Параметры", "Rating"),
            lang.pick("Артикул", "Part number"),
            lang.pick("Кол.", "Qty"),
        ]
        .map(str::to_owned)
        .to_vec(),
        widths: vec![1.0, 4.0, 4.0, 3.0, 3.0, 1.0],
        footer: Some(format!(
            "{} {total}",
            lang.pick("Итого изделий:", "Total devices:")
        )),
        rows,
    }
}

/// Cuts `text` to what fits in `width` units at the size of the table text.
fn fit(text: &str, width: f32) -> String {
    let room = ((width - 10.0) / CHAR_W).max(1.0) as usize;
    if text.chars().count() <= room {
        return text.to_owned();
    }
    let cut: String = text.chars().take(room.saturating_sub(2)).collect();
    format!("{cut}..")
}

/// Rows of one sheet.
fn rows_per_sheet(page: &Page) -> usize {
    ((page.y1() - page.y0() - CAPTION_ROOM - HEAD_H - FOOTER_ROOM) / ROW_H) as usize
}

/// The sheets of a table: the head is repeated on each, the total is on the last one. An empty
/// table has no sheets.
pub fn table_sheets(table: &Table, page: &Page) -> Vec<Sheet> {
    let top = page.y0() + CAPTION_ROOM;
    if table.rows.is_empty() {
        return Vec::new();
    }
    let per = rows_per_sheet(page);
    let pages: Vec<&[Vec<String>]> = table.rows.chunks(per).collect();
    let total_pages = pages.len();
    let sum: f32 = table.widths.iter().sum();
    let width = page.x1() - page.x0();
    let xs: Vec<f32> = table
        .widths
        .iter()
        .scan(page.x0(), |x, w| {
            let at = *x;
            *x += width * w / sum;
            Some(at)
        })
        .collect();
    let col_w = |i: usize| width * table.widths[i] / sum;

    let mut sheets = Vec::new();
    for (index, rows) in pages.iter().enumerate() {
        let mut c = Canvas::new();
        c.bold(
            page.x0() + 2.0,
            page.y0() + 20.0,
            &table.title,
            TEXT_L,
            Align::Left,
            INK,
        );
        let bottom = top + HEAD_H + rows.len() as f32 * ROW_H;
        // The head.
        c.line(&[(page.x0(), top), (page.x1(), top)], THICK, INK);
        for (i, head) in table.head.iter().enumerate() {
            c.bold(
                xs[i] + 5.0,
                top + HEAD_H / 2.0,
                head,
                FONT,
                Align::Left,
                INK,
            );
        }
        c.line(
            &[(page.x0(), top + HEAD_H), (page.x1(), top + HEAD_H)],
            THICK,
            INK,
        );
        // The rows.
        for (r, row) in rows.iter().enumerate() {
            let y = top + HEAD_H + r as f32 * ROW_H;
            for (i, cell) in row.iter().enumerate() {
                let text = fit(cell, col_w(i));
                c.text(xs[i] + 5.0, y + ROW_H / 2.0, &text, FONT, Align::Left, INK);
            }
            c.line(
                &[(page.x0(), y + ROW_H), (page.x1(), y + ROW_H)],
                THIN,
                WEAK,
            );
        }
        // The columns.
        for &x in xs.iter().chain(std::iter::once(&page.x1())) {
            c.line(&[(x, top), (x, bottom)], THIN, INK);
        }
        c.line(&[(page.x0(), bottom), (page.x1(), bottom)], THICK, INK);
        if index + 1 == total_pages {
            if let Some(footer) = &table.footer {
                c.bold(
                    page.x0() + 2.0,
                    bottom + 22.0,
                    footer,
                    FONT,
                    Align::Left,
                    INK,
                );
            }
        }
        sheets.push(Sheet::new(
            if total_pages > 1 {
                format!("{} ({}/{total_pages})", table.title, index + 1)
            } else {
                table.title.clone()
            },
            c,
        ));
    }
    sheets
}

/// The table as CSV: `;` between the cells (a spreadsheet in a Russian locale opens it as is),
/// UTF-8 with a byte order mark, a cell with `;`, `"` or a line break in quotes.
pub fn to_csv(table: &Table) -> String {
    fn cell(text: &str) -> String {
        if text.contains([';', '"', '\n', '\r']) {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text.to_owned()
        }
    }
    let mut out = String::from('\u{feff}');
    for row in std::iter::once(&table.head).chain(&table.rows) {
        let line: Vec<String> = row.iter().map(|c| cell(c)).collect();
        out.push_str(&line.join(";"));
        out.push_str("\r\n");
    }
    out
}
