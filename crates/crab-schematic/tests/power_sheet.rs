//! Power sheets of the panel documentation (`specs/electrical-schematics`, task 4).

use std::path::PathBuf;

use crab_core::{Library, Panel, Project};
use crab_schematic::{build_document, render_pdf, source, Error, Lang};

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

fn pump(panel: &str) -> Project {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(panel).unwrap();
    project
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
rating = "C16, 1P+N"
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
kind = "rcd"
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

fn quoted(text: &str) -> String {
    format!("\"{text}\"")
}

#[test]
fn pump_panel_makes_a_title_sheet_a_power_sheet_and_a_control_sheet() {
    let (doc, nets) = build_document(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    let titles: Vec<&str> = doc.sheets.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Титульный лист",
            "Силовая часть",
            "Цепи управления",
            "Перечень клемм контроллера",
            "Перечень проводов",
            "Спецификация"
        ]
    );
    let body = &doc.sheets[1].body;
    for designation in ["-X1", "-Q1", "-K1", "-M1", "-Q2", "-G1"] {
        assert!(body.contains(&quoted(designation)), "нет {designation}");
    }
    // The control devices belong to the control sheet, not to this one.
    assert!(!body.contains(&quoted("-B1")));
    // Every mains and earth wire has its number written on the sheet.
    for wire in nets.wires.iter().filter(|w| {
        matches!(
            w.class,
            crab_schematic::WireClass::Power | crab_schematic::WireClass::Protective
        )
    }) {
        assert!(
            body.contains(&quoted(&wire.number.to_string())),
            "нет номера {} провода {}",
            wire.number,
            wire.key()
        );
    }
    assert!(body.contains(&quoted("PE")));

    let text = source(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    assert!(text.starts_with("#let u = 0.25mm"));
    assert!(text.contains("paper: \"a3\""));
    for number in ["1 / 6", "2 / 6", "6 / 6"] {
        assert!(text.contains(&quoted(number)), "нет номера листа {number}");
    }
    assert!(text.contains(&quoted("pump")), "метка катушки");
}

#[test]
fn title_sheet_lists_the_sheets_and_says_what_the_controller_does_not_do() {
    let (doc, _) = build_document(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    let title = &doc.sheets[0].body;
    for text in [
        "Электрическая документация щита",
        "pump",
        "Содержание",
        "230 В, 1 фаза",
    ] {
        assert!(title.contains(&quoted(text)), "нет «{text}»");
    }
    // The list: the number and the caption of every sheet, the title sheet first.
    for (i, sheet) in doc.sheets.iter().enumerate() {
        assert!(
            title.contains(&quoted(&(i + 1).to_string())),
            "лист {}",
            i + 1
        );
        assert!(title.contains(&quoted(&sheet.title)), "{}", sheet.title);
    }
    assert!(title.contains("только аппаратно"));
}

#[test]
fn title_sheet_is_in_the_other_language_too() {
    let (doc, _) = build_document(&pump(PUMP_PANEL), &library(), Lang::En).unwrap();
    assert_eq!(doc.sheets[0].title, "Title sheet");
    let title = &doc.sheets[0].body;
    assert!(title.contains(&quoted("Electrical documentation of the panel")));
    assert!(title.contains("hardware only"));
    assert!(!title.contains("Содержание"));
}

#[test]
fn captions_follow_the_language() {
    let (doc, _) = build_document(&pump(PUMP_PANEL), &library(), Lang::En).unwrap();
    assert_eq!(doc.sheets[1].title, "Power circuit");
    let text = source(&pump(PUMP_PANEL), &library(), Lang::En).unwrap();
    assert!(text.contains(&quoted("Project")));
    assert!(!text.contains("Проект"));
}

#[test]
fn the_same_panel_gives_the_same_source() {
    let a = source(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    let b = source(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    assert_eq!(a, b);
}

#[test]
fn many_branches_are_split_between_sheets() {
    let mut panel = String::from("[[device]]\nid = \"X1\"\nkind = \"feed\"\n");
    for i in 1..=12 {
        panel.push_str(&format!(
            "[[device]]\nid = \"Q{i}\"\nkind = \"breaker\"\nload = \"M{i}\"\n\
             [[device]]\nid = \"M{i}\"\nkind = \"motor\"\n"
        ));
    }
    let (doc, _) = build_document(&pump(&panel), &library(), Lang::Ru).unwrap();
    let power: Vec<&str> = doc
        .sheets
        .iter()
        .map(|s| s.title.as_str())
        .filter(|t| t.starts_with("Силовая часть"))
        .collect();
    assert_eq!(
        power,
        ["Силовая часть (1/2)", "Силовая часть (2/2)"],
        "12 ветвей на 230 В не помещаются на один силовой лист"
    );
    assert_eq!(doc.sheets[1].title, "Силовая часть (1/2)");
    assert_eq!(doc.sheets[2].title, "Силовая часть (2/2)");
    // Every device is on exactly one sheet.
    for i in 1..=12 {
        for designation in [format!("-Q{i}"), format!("-M{i}")] {
            let count = doc
                .sheets
                .iter()
                .filter(|s| s.body.contains(&quoted(&designation)))
                .count();
            assert_eq!(count, 1, "{designation}");
        }
    }
}

#[test]
fn chain_that_is_too_long_is_cut_and_the_cuts_are_marked() {
    let mut panel = String::from("[[device]]\nid = \"X1\"\nkind = \"feed\"\n");
    for i in 1..=12 {
        let next = if i < 12 {
            format!("Q{}", i + 1)
        } else {
            "M1".to_owned()
        };
        panel.push_str(&format!(
            "[[device]]\nid = \"Q{i}\"\nkind = \"breaker\"\nload = \"{next}\"\n"
        ));
    }
    panel.push_str("[[device]]\nid = \"M1\"\nkind = \"motor\"\n");
    let (doc, nets) = build_document(&pump(&panel), &library(), Lang::Ru).unwrap();
    // 13 devices, 5 rows on a sheet: three segments in three columns of one sheet.
    let power: Vec<_> = doc
        .sheets
        .iter()
        .filter(|s| s.title.starts_with("Силовая часть"))
        .collect();
    assert_eq!(power.len(), 1);
    for i in 1..=12 {
        assert!(power[0].body.contains(&quoted(&format!("-Q{i}"))), "Q{i}");
    }
    assert!(power[0].body.contains(&quoted("-M1")));
    // The segments begin at Q6 and Q11. Each cut is marked on both sides: where the chain goes on,
    // and where it comes from. The marks carry the real place of the device they name.
    for (before, after) in [("Q5", "Q6"), ("Q10", "Q11")] {
        let place = &doc.power_at[after];
        // The references are on the sheet they point to: the sheet is left out (5.8).
        let next = format!("далее -{after} {}", place.from_sheet(place.sheet));
        let there = &doc.power_at[before];
        let from = format!("от -{before} {}", there.from_sheet(there.sheet));
        assert!(power[0].body.contains(&quoted(&next)), "нет «{next}»");
        assert!(power[0].body.contains(&quoted(&from)), "нет «{from}»");
    }
    // The wire across the cut keeps one number on both sides.
    let cut = nets
        .wires
        .iter()
        .find(|w| w.from.device == "Q5" && w.to.device == "Q6")
        .expect("провод через разрыв");
    assert!(
        power[0]
            .body
            .matches(&quoted(&cut.number.to_string()))
            .count()
            >= 2
    );
}

#[test]
fn zones_of_the_power_devices_are_the_zones_where_they_are_drawn() {
    let mut panel = String::from("[[device]]\nid = \"X1\"\nkind = \"feed\"\n");
    for i in 1..=8 {
        let next = if i < 8 {
            format!("Q{}", i + 1)
        } else {
            "M1".to_owned()
        };
        panel.push_str(&format!(
            "[[device]]\nid = \"Q{i}\"\nkind = \"breaker\"\nload = \"{next}\"\n"
        ));
    }
    panel.push_str("[[device]]\nid = \"M1\"\nkind = \"motor\"\n");
    let (doc, _) = build_document(&pump(&panel), &library(), Lang::Ru).unwrap();
    for id in ["Q1", "Q5", "Q6", "Q8", "M1"] {
        let place = &doc.power_at[id];
        let body = &doc.sheets[place.sheet - 1].body;
        // The designation is drawn 2 units right of the column and 38 below the top terminals;
        // the contacts are 46 right of the column and 39 below the top.
        let label = format!("\"-{id}\"");
        let line = body
            .lines()
            .find(|l| l.starts_with("TB(") && l.ends_with(&format!(", {label})")))
            .unwrap_or_else(|| panic!("нет обозначения -{id} на листе {}", place.sheet));
        let nums: Vec<f32> = line["TB(".len()..]
            .split(", ")
            .take(2)
            .map(|s| s.parse().unwrap())
            .collect();
        let (x, y) = (nums[0] - 2.0 + 46.0, nums[1] - 38.0 + 39.0);
        assert_eq!(
            place.zone,
            crab_schematic::Page::default().zone(x, y),
            "{id}"
        );
    }
}

#[test]
fn panel_without_power_circuits_has_no_power_sheet() {
    let panel = "[[device]]\nid = \"B1\"\nkind = \"sensor\"\nbind = \"sensor.out\"\n";
    let (doc, _) = build_document(&pump(panel), &library(), Lang::Ru).unwrap();
    let titles: Vec<&str> = doc.sheets.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Титульный лист",
            "Цепи управления",
            "Перечень клемм контроллера",
            "Перечень проводов",
            "Спецификация"
        ]
    );
}

#[test]
fn nothing_to_draw_means_no_sheets_not_even_a_title() {
    let panel = "[[device]]\nid = \"X1\"\nkind = \"feed\"\n";
    let (doc, _) = build_document(&pump(panel), &library(), Lang::Ru).unwrap();
    assert!(doc.sheets.is_empty());
}

#[test]
fn panel_without_bound_devices_has_no_control_sheet() {
    let panel =
        "[[device]]\nid = \"X1\"\nkind = \"feed\"\n[[device]]\nid = \"M1\"\nkind = \"motor\"\n";
    let (doc, _) = build_document(&pump(panel), &library(), Lang::Ru).unwrap();
    assert!(doc.sheets.iter().all(|s| s.title != "Цепи управления"));
}

/// `CRAB_PANEL_SRC_OUT=file.typ cargo test` keeps the markup of the pump panel for a look.
#[test]
fn markup_can_be_kept_for_a_look() {
    if let Some(out) = std::env::var_os("CRAB_PANEL_SRC_OUT") {
        let text = source(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
        std::fs::write(out, text).unwrap();
    }
    // The same for a chain of 13 devices, which is cut.
    if let Some(out) = std::env::var_os("CRAB_PANEL_LONG_OUT") {
        let mut panel = String::from("[[device]]\nid = \"X1\"\nkind = \"feed\"\n");
        for i in 1..=12 {
            let next = if i < 12 {
                format!("Q{}", i + 1)
            } else {
                "M1".to_owned()
            };
            panel.push_str(&format!(
                "[[device]]\nid = \"Q{i}\"\nkind = \"breaker\"\nload = \"{next}\"\n"
            ));
        }
        panel.push_str("[[device]]\nid = \"M1\"\nkind = \"motor\"\n");
        let text = source(&pump(&panel), &library(), Lang::Ru).unwrap();
        std::fs::write(out, text).unwrap();
    }
}

#[test]
fn pdf_is_made_when_the_helper_is_there() {
    match render_pdf(&pump(PUMP_PANEL), &library(), Lang::Ru) {
        Ok(pdf) => {
            assert!(pdf.starts_with(b"%PDF"));
            if let Some(out) = std::env::var_os("CRAB_PANEL_PDF_OUT") {
                std::fs::write(out, &pdf).unwrap();
            }
        }
        // The helper is built by `cargo build -p crab-pdf-render`; without it only the markup is checked.
        Err(Error::Pdf(crab_pdf::PdfError::NoHelper)) => {}
        Err(e) => panic!("{e}"),
    }
}
