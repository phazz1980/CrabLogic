//! Terminal list, wire list and bill of materials (`specs/electrical-schematics`, task 7).

use std::path::PathBuf;

use crab_core::{Library, Panel, Project};
use crab_schematic::tables::{table_sheets, to_csv};
use crab_schematic::{build_document, Lang, Page, Table};

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn library() -> Library {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    lib
}

const PUMP_PANEL: &str = r#"
supply = "ac230"

[[device]]
id = "X1"
kind = "feed"
rating = "230 В, 16 А"

[[device]]
id = "Q1"
kind = "breaker"
rating = "C16"
part = "ABB S201-C16"
load = "K1"

[[device]]
id = "K1"
kind = "contactor"
rating = "9 А"
bind = "pump"
load = "M1"

[[device]]
id = "M1"
kind = "motor"
rating = "0,75 кВт"

[[device]]
id = "Q2"
kind = "breaker"
rating = "C16"
part = "ABB S201-C16"
load = "G1"

[[device]]
id = "G1"
kind = "psu"
rating = "24 В, 5 А"

[[device]]
id = "B1"
kind = "sensor"
bind = "sensor.out"
"#;

fn pump(panel: &str) -> Project {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(panel).unwrap();
    project
}

fn document(panel: &str, lang: Lang) -> (crab_schematic::Document, crab_schematic::Nets) {
    build_document(&pump(panel), &library(), lang).unwrap()
}

/// A reader of the CSV that `to_csv` writes: the BOM, `;`, quotes, line ends.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').expect("метка UTF-8");
    let (mut rows, mut row, mut cell) = (Vec::new(), Vec::new(), String::new());
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            (true, '"') => quoted = false,
            (true, c) => cell.push(c),
            (false, '"') => quoted = true,
            (false, ';') => row.push(std::mem::take(&mut cell)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => cell.push(c),
        }
    }
    assert!(
        cell.is_empty() && row.is_empty(),
        "файл кончается не переводом строки"
    );
    rows
}

#[test]
fn terminal_list_names_the_terminal_the_signal_the_device_and_the_wire() {
    let (doc, nets) = document(PUMP_PANEL, Lang::Ru);
    let t = &doc.tables.terminals;
    assert_eq!(
        t.head,
        [
            "Клемма",
            "Вход или выход проекта",
            "Направление",
            "Устройство",
            "Провод №"
        ]
    );
    let number = |device: &str| {
        nets.board_terminals
            .iter()
            .find(|t| t.device == device)
            .unwrap()
            .wire
            .to_string()
    };
    assert_eq!(
        t.rows,
        [
            ["A1:IO26", "pump", "выход", "-K1", &number("K1")],
            ["A1:IO34", "sensor", "вход", "-B1", &number("B1")],
        ]
    );
    assert_eq!(t.footer.as_deref(), Some("Клемм: 2"));
}

#[test]
fn wire_list_has_every_wire_by_number() {
    let (doc, nets) = document(PUMP_PANEL, Lang::Ru);
    let w = &doc.tables.wires;
    assert_eq!(w.rows.len(), nets.wires.len());
    let numbers: Vec<u32> = w.rows.iter().map(|r| r[0].parse().unwrap()).collect();
    let mut sorted = numbers.clone();
    sorted.sort_unstable();
    assert_eq!(numbers, sorted, "по возрастанию номеров");
    // A row says what the wire is and where it goes.
    let row = w
        .rows
        .iter()
        .find(|r| r[2] == "K1:2" && r[3] == "M1:L")
        .unwrap();
    assert_eq!(row[1], "Силовой");
    assert!(w
        .rows
        .iter()
        .any(|r| r[1] == "Защитный PE" && r[2] == "X1:PE"));
    assert!(w.rows.iter().any(|r| r[1] == "Управление 24 В"));
    assert!(w.rows.iter().any(|r| r[1] == "Сигнал" && r[3] == "A1:IO34"));
}

#[test]
fn bom_groups_equal_devices_and_the_total_is_the_number_of_devices() {
    let project = pump(PUMP_PANEL);
    let (doc, _) = document(PUMP_PANEL, Lang::Ru);
    let bom = &doc.tables.bom;
    // Q1 and Q2 are the same breaker: one line with two devices.
    let breakers = bom
        .rows
        .iter()
        .find(|r| r[2] == "Автоматический выключатель")
        .unwrap();
    assert_eq!(breakers[1], "-Q1, -Q2");
    assert_eq!(breakers[3], "C16");
    assert_eq!(breakers[4], "ABB S201-C16");
    assert_eq!(breakers[5], "2");
    // The totals: the sum of the quantities is the number of devices.
    let sum: usize = bom
        .rows
        .iter()
        .map(|r| r[5].parse::<usize>().unwrap())
        .sum();
    assert_eq!(sum, project.panel.devices.len());
    assert_eq!(
        bom.footer.as_deref(),
        Some(format!("Итого изделий: {sum}").as_str())
    );
    // The positions are numbered 1, 2, 3...
    for (i, row) in bom.rows.iter().enumerate() {
        assert_eq!(row[0], (i + 1).to_string());
    }
}

#[test]
fn different_rating_or_part_number_is_a_different_line() {
    let panel = r#"
[[device]]
id = "Q1"
kind = "breaker"
rating = "C10"
[[device]]
id = "Q2"
kind = "breaker"
rating = "C16"
[[device]]
id = "Q3"
kind = "breaker"
rating = "C16"
part = "S201"
[[device]]
id = "Q4"
kind = "breaker"
rating = "C16"
part = "S201"
"#;
    let bom = &document(panel, Lang::Ru).0.tables.bom;
    let lines: Vec<(&str, &str)> = bom
        .rows
        .iter()
        .map(|r| (r[1].as_str(), r[5].as_str()))
        .collect();
    assert_eq!(lines, [("-Q1", "1"), ("-Q2", "1"), ("-Q3, -Q4", "2")]);
}

#[test]
fn csv_reads_back_to_the_same_table_even_with_awkward_cells() {
    let panel = r#"
[[device]]
id = "Q1"
kind = "breaker"
rating = "C16; \"3P\""
part = "ABB;\nS201"
"#;
    let bom = document(panel, Lang::Ru).0.tables.bom;
    let text = to_csv(&bom);
    assert!(text.starts_with('\u{feff}'));
    assert!(text.contains("\r\n"));
    let rows = parse_csv(&text);
    let mut expected = vec![bom.head.clone()];
    expected.extend(bom.rows.clone());
    assert_eq!(rows, expected);
    assert_eq!(rows[1][3], "C16; \"3P\"");
    assert_eq!(rows[1][4], "ABB;\nS201");
}

#[test]
fn csv_of_the_other_tables_reads_back_too() {
    let (doc, _) = document(PUMP_PANEL, Lang::Ru);
    for table in [&doc.tables.terminals, &doc.tables.wires, &doc.tables.bom] {
        let rows = parse_csv(&to_csv(table));
        assert_eq!(rows.len(), table.rows.len() + 1, "{}", table.title);
        assert_eq!(rows[0], table.head);
        assert_eq!(&rows[1..], table.rows.as_slice());
        // The total line is for the sheet, not for the spreadsheet.
        assert!(rows.iter().all(|r| r.len() == table.head.len()));
    }
}

#[test]
fn a_long_table_goes_to_several_sheets_with_the_head_on_each() {
    let mut panel = String::new();
    for i in 1..=70 {
        panel.push_str(&format!(
            "[[device]]\nid = \"Q{i}\"\nkind = \"breaker\"\nrating = \"C{i}\"\n"
        ));
    }
    let bom = document(&panel, Lang::Ru).0.tables.bom;
    assert_eq!(bom.rows.len(), 70);
    let sheets = table_sheets(&bom, &Page::default());
    assert_eq!(sheets.len(), 2, "70 строк не помещаются на один лист");
    assert_eq!(sheets[0].title, "Спецификация (1/2)");
    assert_eq!(sheets[1].title, "Спецификация (2/2)");
    for sheet in &sheets {
        assert!(sheet.body.contains("\"Артикул\""), "шапка на каждом листе");
    }
    // Every device is in the table once; the total is only under the last page.
    for i in 1..=70 {
        let on = sheets
            .iter()
            .filter(|s| s.body.contains(&format!("\"-Q{i}\"")))
            .count();
        assert_eq!(on, 1, "Q{i}");
    }
    assert!(!sheets[0].body.contains("Итого изделий"));
    assert!(sheets[1].body.contains("Итого изделий: 70"));
}

#[test]
fn table_sheets_come_after_the_schemes_and_are_in_the_contents() {
    let (doc, _) = document(PUMP_PANEL, Lang::Ru);
    let titles: Vec<&str> = doc.sheets.iter().map(|s| s.title.as_str()).collect();
    let last = &titles[titles.len() - 3..];
    assert_eq!(
        last,
        [
            "Перечень клемм контроллера",
            "Перечень проводов",
            "Спецификация"
        ]
    );
    for title in last {
        assert!(
            doc.sheets[0].body.contains(&format!("\"{title}\"")),
            "{title}"
        );
    }
}

#[test]
fn tables_are_in_the_other_language_too() {
    let (doc, _) = document(PUMP_PANEL, Lang::En);
    assert_eq!(doc.tables.bom.title, "Bill of materials");
    assert_eq!(doc.tables.wires.head[0], "Wire no.");
    assert_eq!(doc.tables.terminals.rows[0][2], "output");
    assert!(doc
        .tables
        .bom
        .rows
        .iter()
        .any(|r| r[2] == "Circuit breaker"));
    assert!(doc
        .tables
        .bom
        .footer
        .as_deref()
        .unwrap()
        .starts_with("Total devices:"));
}

#[test]
fn empty_tables_have_no_sheets() {
    let table = Table {
        title: "x".into(),
        head: vec!["a".into()],
        rows: Vec::new(),
        widths: vec![1.0],
        footer: None,
    };
    assert!(table_sheets(&table, &Page::default()).is_empty());
}
