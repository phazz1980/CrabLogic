//! The title sheet: what the documentation is about and the list of its sheets.

use crab_core::{Project, Supply};

use crate::canvas::{Align, Canvas, INK, TEXT, TEXT_L, TEXT_XL, WEAK};
use crate::doc::{Format, Page, Sheet};
use crate::lang::Lang;
use crate::nets::Nets;

const ROW_H: f32 = 25.0;

fn supply_text(supply: Supply, lang: Lang) -> &'static str {
    match supply {
        Supply::Dc24 => lang.pick("24 В постоянного тока", "24 V DC"),
        Supply::Ac230 => lang.pick("230 В, 1 фаза", "230 V, single phase"),
        Supply::Ac400 => lang.pick("400 В, 3 фазы", "400 V, three phase"),
    }
}

/// The title sheet. `titles` are the captions of the sheets that follow it, in order; the title
/// sheet itself is number 1 and the first of them is number 2.
pub fn title_sheet(
    project: &Project,
    nets: &Nets,
    titles: &[&str],
    lang: Lang,
    page: &Page,
) -> Sheet {
    // The vertical offsets are those of A3; a sheet of A4 is shorter.
    let k = if page.format == Format::A4 { 0.7 } else { 1.0 };
    let column_w = if page.format == Format::A4 {
        480.0
    } else {
        700.0
    };
    let mut c = Canvas::new();
    let mid = (page.x0() + page.x1()) / 2.0;
    c.bold(
        mid,
        page.y0() + 100.0 * k,
        lang.pick(
            "Электрическая документация щита",
            "Electrical documentation of the panel",
        ),
        TEXT_XL,
        Align::Center,
        INK,
    );
    c.bold(
        mid,
        page.y0() + 160.0 * k,
        &project.display_name(),
        TEXT_L,
        Align::Center,
        INK,
    );

    // What the panel is.
    let board = &project.config.board;
    let board_name = board.model.as_deref().unwrap_or(&board.fqbn);
    let facts = [
        (
            lang.pick("Питание", "Supply"),
            supply_text(project.panel.supply, lang).to_owned(),
        ),
        (lang.pick("Контроллер", "Controller"), board_name.to_owned()),
        (
            lang.pick("Устройств в щите", "Devices"),
            project.panel.devices.len().to_string(),
        ),
        (lang.pick("Проводов", "Wires"), nets.wires.len().to_string()),
    ];
    for (i, (name, value)) in facts.iter().enumerate() {
        let y = page.y0() + 230.0 * k + i as f32 * 25.0 * k;
        c.text(mid - 10.0, y, name, TEXT, Align::Right, WEAK);
        c.text(mid + 10.0, y, value, TEXT, Align::Left, INK);
    }

    // The list of the sheets, in columns.
    let top = page.y0() + 400.0 * k;
    c.bold(
        page.x0() + 60.0,
        top - 35.0,
        lang.pick("Содержание", "Contents"),
        TEXT_L,
        Align::Left,
        INK,
    );
    // Lines of the list of sheets in one column.
    let rows_in_column = (((page.y1() - top - 40.0) / ROW_H) as usize).max(1);
    let rows =
        std::iter::once(lang.pick("Титульный лист", "Title sheet")).chain(titles.iter().copied());
    for (i, title) in rows.enumerate() {
        let (column, row) = (i / rows_in_column, i % rows_in_column);
        let x = page.x0() + 60.0 + column as f32 * column_w;
        let y = top + row as f32 * ROW_H;
        c.text(x + 30.0, y, &(i + 1).to_string(), TEXT, Align::Right, WEAK);
        c.text(x + 50.0, y, title, TEXT, Align::Left, INK);
    }

    // Emergency functions are a matter of hardware only.
    c.text(
        page.x0() + 60.0,
        page.y1() - 15.0,
        lang.pick(
            "Контроллер не выполняет функций аварийной защиты: аварийный останов и блокировки выполняются только аппаратно.",
            "The controller does not perform emergency protection: emergency stop and interlocks are hardware only.",
        ),
        TEXT,
        Align::Left,
        WEAK,
    );
    Sheet::new(lang.pick("Титульный лист", "Title sheet").to_owned(), c)
}
