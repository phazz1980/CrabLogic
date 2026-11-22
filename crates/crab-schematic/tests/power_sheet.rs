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
    for i in 1..=16 {
        let next = if i < 16 {
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
    // The first three breakers stand across the entering lines; the other 14 devices, 8 rows on a
    // sheet: two segments in two columns of one sheet.
    let power: Vec<_> = doc
        .sheets
        .iter()
        .filter(|s| s.title.starts_with("Силовая часть"))
        .collect();
    assert_eq!(power.len(), 1);
    for i in 1..=16 {
        assert!(power[0].body.contains(&quoted(&format!("-Q{i}"))), "Q{i}");
    }
    assert!(power[0].body.contains(&quoted("-M1")));
    // The segments begin at Q4 and Q12. Each cut is marked on both sides: where the chain goes on,
    // and where it comes from. The marks carry the real place of the device they name.
    {
        let (before, after) = ("Q11", "Q12");
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
        .find(|w| w.from.device == "Q11" && w.to.device == "Q12")
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

/// A terminal block in the chain and auxiliary contacts do not stop the sheets from being made
/// (they are drawn by the later tasks of `specs/panel-aux`).
#[test]
fn terminal_blocks_and_contacts_do_not_break_the_sheets() {
    let panel = r#"
supply = "ac230"
[[device]]
id = "X1"
kind = "feed"
load = "Q1"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"
[[device]]
id = "Q1_1"
kind = "aux"
of = "Q1"
bind = "sensor"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "X1.1"
[[device]]
id = "K1_1"
kind = "aux"
of = "K1"
series = "K1"
[[device]]
id = "X1.1"
kind = "terminal"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
"#;
    let (doc, nets) = build_document(&pump(panel), &library(), Lang::Ru).unwrap();
    assert!(!doc.sheets.is_empty());
    assert!(nets.wires.iter().any(|w| w.key() == "X1.1:PE>M1:PE"));
}

/// A bus that goes on to another sheet ends with its name and the reference of the sheet, and
/// starts there with the reference back (`specs/panel-aux`, task 4).
#[test]
fn a_bus_refers_to_the_sheets_it_goes_on_to_and_comes_from() {
    let mut panel = String::from("supply = \"ac230\"\n[[device]]\nid = \"X1\"\nkind = \"feed\"\n");
    for n in 1..=16 {
        panel.push_str(&format!("[[device]]\nid = \"Q{n}\"\nkind = \"breaker\"\n"));
    }
    let (doc, _) = build_document(&pump(&panel), &library(), Lang::Ru).unwrap();
    let power: Vec<&crab_schematic::Sheet> = doc
        .sheets
        .iter()
        .filter(|s| s.title.starts_with("Силовая часть"))
        .collect();
    assert!(power.len() >= 2, "the 16 breakers fit one sheet");
    let refs = |body: &str| -> Vec<String> {
        body.split('"')
            .filter(|t| t.starts_with('/') && t.contains('.'))
            .map(str::to_owned)
            .collect()
    };
    // The first sheet only goes on, the last only comes from, those between do both.
    let first = refs(&power[0].body);
    assert!(
        first.iter().any(|r| r.starts_with("/3.")),
        "{first:?} (the sheet after the title sheet is 2, the next 3)"
    );
    let last = refs(&power[power.len() - 1].body);
    assert!(
        last.iter()
            .any(|r| r.starts_with(&format!("/{}.", power.len()))),
        "{last:?}"
    );
    // The name of the conductor stands over the end of the line.
    assert!(power[0].body.matches(&quoted("L")).count() >= 2);
}

const CONTACTS_PANEL: &str = r#"
supply = "ac400"
[[device]]
id = "X1"
kind = "feed"
load = "Q1"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"
[[device]]
id = "Q1_1"
kind = "aux"
of = "Q1"
[[device]]
id = "Q1_2"
kind = "aux"
of = "Q1"
form = "nc"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "X1.1"
[[device]]
id = "X1.1"
kind = "terminal"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
"#;

/// The power sheet draws the auxiliary contacts of the breaker with their terminals, the row of
/// terminals with the sheet number, and the names of the conductors at the motor.
#[test]
fn auxiliary_contacts_terminals_and_motor_names_are_drawn() {
    let (doc, _) = build_document(&pump(CONTACTS_PANEL), &library(), Lang::Ru).unwrap();
    let body = &doc.sheets[1].body;
    for text in [
        "13", "14", "21", "22", "2.1", "2.4", "2.5PE", "U", "V", "W", "-X1.1",
    ] {
        assert!(body.contains(&quoted(text)), "нет надписи {text}");
    }
}

#[test]
fn the_sheets_with_contacts_and_terminals_make_a_pdf() {
    if let Ok(pdf) = render_pdf(&pump(CONTACTS_PANEL), &library(), Lang::Ru) {
        assert!(pdf.starts_with(b"%PDF"));
        if let Some(out) = std::env::var_os("CRAB_PANEL_PDF_OUT") {
            std::fs::write(out, &pdf).unwrap();
        }
    }
}

/// The motor behind a terminal block takes its earth from the block's earth terminal, up the
/// column, and has an earth terminal of its own next to the poles instead of an earth sign.
#[test]
fn the_earth_of_a_motor_behind_a_terminal_block_runs_up() {
    use crab_schematic::scene::ItemKind;
    let (doc, _) = build_document(&pump(CONTACTS_PANEL), &library(), Lang::Ru).unwrap();
    let sheet = &doc.sheets[1];
    assert!(sheet.body.contains(&quoted("PE")));
    let wire = sheet
        .items
        .iter()
        .find_map(|i| match &i.kind {
            ItemKind::Wire { key, from, to } if key == "X1.1:PE>M1:PE" => Some((from, to)),
            _ => None,
        })
        .expect("the earth wire between the terminal block and the motor is an item");
    assert_eq!(
        (wire.0.device.as_str(), wire.0.port.as_str()),
        ("X1.1", "pe")
    );
    assert_eq!((wire.1.device.as_str(), wire.1.port.as_str()), ("M1", "pe"));
    // The motor has no earth sign below: its only `pe` port is the one at the top.
    let ports: Vec<_> = sheet
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Symbol { device, ports, .. } if device == "M1" => Some(ports),
            _ => None,
        })
        .flatten()
        .filter(|p| p.name == "pe")
        .collect();
    assert_eq!(ports.len(), 1);
}

/// The references of the sheets are links in the PDF: a click goes to the sheet they name.
#[test]
fn the_references_are_links_in_the_typst_source_and_in_the_pdf() {
    let text = source(&pump(CONTACTS_PANEL), &library(), Lang::Ru).unwrap();
    assert!(text.contains("link((page: "), "no link in the source");
    // A link never names a page that is not there.
    let total = text.matches("#pagebreak()").count() + 1;
    for part in text.split("link((page: ").skip(1) {
        let page: usize = part.split(',').next().unwrap().parse().unwrap();
        assert!((1..=total).contains(&page), "page {page} of {total}");
    }
    if let Ok(pdf) = render_pdf(&pump(CONTACTS_PANEL), &library(), Lang::Ru) {
        assert!(
            pdf.windows(5).any(|w| w == b"/Link"),
            "the PDF has no link annotations"
        );
    }
}

/// The auxiliary contacts of a breaker stand right of it on the power sheet; those of a contactor
/// go under its coil on the control sheet, so the power sheet does not draw them.
#[test]
fn contacts_of_a_contactor_are_not_drawn_beside_it() {
    let panel = r#"
supply = "ac230"
[[device]]
id = "X1"
kind = "feed"
load = "Q1"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"
[[device]]
id = "K1_1"
kind = "aux"
of = "K1"
form = "no"
[[device]]
id = "M1"
kind = "motor"
"#;
    let (doc, _) = build_document(&pump(panel), &library(), Lang::Ru).unwrap();
    let body = &doc.sheets[1].body;
    // No contact terminals `13` / `14` beside the contactor.
    assert!(!body.contains(&quoted("13")) && !body.contains(&quoted("14")));
    // The same contact on the breaker is drawn.
    let with_breaker = panel.replace("of = \"K1\"", "of = \"Q1\"");
    let (doc, _) = build_document(&pump(&with_breaker), &library(), Lang::Ru).unwrap();
    assert!(doc.sheets[1].body.contains(&quoted("13")));
}

/// A normally closed contact is the open one plus the hook of the fixed contact the blade rests
/// on: a short horizontal line joining the end of the blade to the line below (not a straight
/// wire, which looks like no contact at all).
#[test]
fn a_normally_closed_contact_has_the_hook_under_its_blade() {
    use crab_schematic::Op;
    let hooks = |form: &str| -> usize {
        let panel = format!(
            "supply = \"ac230\"\n[[device]]\nid = \"X1\"\nkind = \"feed\"\nload = \"Q1\"\n\
             [[device]]\nid = \"Q1\"\nkind = \"breaker\"\n\
             [[device]]\nid = \"Q1_1\"\nkind = \"aux\"\nof = \"Q1\"\nform = \"{form}\"\n"
        );
        let (doc, _) = build_document(&pump(&panel), &library(), Lang::Ru).unwrap();
        doc.sheets[1]
            .ops
            .iter()
            .filter(|op| match op {
                Op::Line {
                    points,
                    dashed: false,
                    ..
                } => {
                    points.len() == 2
                        && points[0].1 == points[1].1
                        && (points[0].0 - points[1].0).abs() == 5.0
                }
                _ => false,
            })
            .count()
    };
    assert_eq!(hooks("no"), 0);
    assert_eq!(hooks("nc"), 1);
}

/// A contactor shows no coil beside its contacts: where the coil is drawn stands under its
/// designation, and the coil itself is on the control sheet.
#[test]
fn a_contactor_has_no_coil_beside_the_contacts() {
    use crab_schematic::Op;
    let (doc, _) = build_document(&pump(PUMP_PANEL), &library(), Lang::Ru).unwrap();
    let power = &doc.sheets[1];
    // The coil was a 20 x 40 rectangle next to the poles.
    assert!(
        !power.ops.iter().any(|op| matches!(
            op,
            Op::Rect { w, h, .. } if (*w, *h) == (20.0, 40.0)
        )),
        "the coil is still drawn on the power sheet"
    );
    // The reference to the coil is a caption of the contactor.
    assert!(power.body.contains("\"/3.") || power.body.contains("\"/4."));
    let control = doc
        .sheets
        .iter()
        .find(|s| s.title.starts_with("Цепи управления"))
        .unwrap();
    assert!(
        control.body.contains(&quoted("-K1")),
        "the coil is on the control sheet"
    );
}

/// A motor has no output; a temperature sensor can be put at it: it is drawn beside the motor
/// with its designation and the place on the control sheet.
#[test]
fn a_motor_shows_its_temperature_sensor() {
    let panel = "supply = \"ac230\"
[[device]]
id = \"X1\"
kind = \"feed\"
load = \"M1\"
         [[device]]
id = \"M1\"
kind = \"motor\"
         [[device]]
id = \"B1\"
kind = \"sensor\"
of = \"M1\"
bind = \"pressure\"
";
    let (doc, _) = build_document(&pump(panel), &library(), Lang::Ru).unwrap();
    assert!(doc.sheets[1].body.contains(&quoted("-B1")));
    // Without the sensor nothing of the kind is drawn.
    let plain = "supply = \"ac230\"
[[device]]
id = \"X1\"
kind = \"feed\"
load = \"M1\"
         [[device]]
id = \"M1\"
kind = \"motor\"
";
    let (doc, _) = build_document(&pump(plain), &library(), Lang::Ru).unwrap();
    assert!(!doc.sheets[1].body.contains(&quoted("-B1")));
}

#[test]
fn the_first_devices_of_a_single_circuit_stand_across_the_entering_lines() {
    use crab_schematic::ItemKind;
    let panel = r#"
supply = "ac230"
[[device]]
id = "X1"
kind = "feed"
[[device]]
id = "S1"
kind = "switch"
load = "F1"
[[device]]
id = "F1"
kind = "fuse"
load = "Q1"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
"#;
    let project = pump(panel);
    let (doc, _) = build_document(&project, &library(), Lang::Ru).unwrap();
    let origin = |id: &str| {
        doc.sheets[1]
            .items
            .iter()
            .find_map(|i| match &i.kind {
                ItemKind::Symbol {
                    view: "power",
                    device,
                    origin,
                    ..
                } if device == id => Some(*origin),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no symbol of {id}"))
    };
    let (s1, f1, q1, k1) = (origin("S1"), origin("F1"), origin("Q1"), origin("K1"));
    // One column, the first device lowest; the contactor stands in a column to the right.
    assert_eq!(s1.0, f1.0);
    assert_eq!(f1.0, q1.0);
    assert!(s1.1 > f1.1 && f1.1 > q1.1);
    assert!(k1.0 > s1.0);
}

#[test]
fn several_devices_on_one_source_branch_at_a_dot() {
    use crab_schematic::ItemKind;
    let panel = r#"
supply = "ac230"
[[device]]
id = "X1"
kind = "feed"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"
[[device]]
id = "Q2"
kind = "breaker"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
[[device]]
id = "K2"
kind = "contactor"
from = "K1"
load = "M2"
[[device]]
id = "M2"
kind = "motor"
"#;
    let project = pump(panel);
    let (doc, nets) = build_document(&project, &library(), Lang::Ru).unwrap();
    // K2 hangs on the output of K1 as well as M1 does.
    let wire = nets
        .wires
        .iter()
        .find(|w| w.to.device == "K2")
        .expect("wire to K2");
    assert_eq!(wire.from.device, "K1");
    // K2 stands in a column of its own, and its wire is one item with a dot on it.
    let sheet = &doc.sheets[1];
    let origin = |id: &str| {
        sheet
            .items
            .iter()
            .find_map(|i| match &i.kind {
                ItemKind::Symbol {
                    view: "power",
                    device,
                    origin,
                    ..
                } if device == id => Some(*origin),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no symbol of {id}"))
    };
    assert_ne!(origin("K2").0, origin("K1").0);
    let item = sheet
        .items
        .iter()
        .find(|i| matches!(&i.kind, ItemKind::Wire { key, .. } if *key == wire.key()))
        .expect("wire item");
    assert!(item.ops.len() >= 2);
    let report = crab_core::check_panel(&project, &library());
    assert!(report
        .diagnostics
        .iter()
        .all(|d| !d.message.contains("такого источника")));
}

#[test]
fn a_new_device_stays_off_the_sheet_until_it_is_connected() {
    let panel = r#"
supply = "ac230"
[[device]]
id = "X1"
kind = "feed"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"
[[device]]
id = "K1"
kind = "contactor"
[[device]]
id = "Q2"
kind = "breaker"
free = true
"#;
    let mut project = pump(panel);
    let (doc, nets) = build_document(&project, &library(), Lang::Ru).unwrap();
    assert!(!doc.power_at.contains_key("Q2"));
    assert!(nets.wires.iter().all(|w| w.to.device != "Q2"));
    // Connected straight to the feed, it appears.
    project.panel.reconnect_power("Q2", None).unwrap();
    assert!(!project.panel.device("Q2").unwrap().free);
    let (doc, nets) = build_document(&project, &library(), Lang::Ru).unwrap();
    assert!(doc.power_at.contains_key("Q2"));
    assert!(nets.wires.iter().any(|w| w.to.device == "Q2"));
}

/// The first line of the bus `k` of the first power sheet: its points.
fn bus_points(doc: &crab_schematic::Document, k: usize) -> Vec<(f32, f32)> {
    use crab_schematic::{ItemKind, Op};
    let sheet = &doc.sheets[1];
    let item = sheet
        .items
        .iter()
        .find(|i| matches!(i.kind, ItemKind::Bus { index: 0, k: kk } if kk == k))
        .expect("bus item");
    item.ops
        .iter()
        .find_map(|&o| match &sheet.ops[o] {
            Op::Line { points, .. } => Some(points.clone()),
            _ => None,
        })
        .expect("bus line")
}

const THREE_POLE: &str = r#"
supply = "ac400"
[[device]]
id = "X1"
kind = "feed"
[[device]]
id = "Q1"
kind = "breaker"
poles = 3
load = "K1"
[[device]]
id = "K1"
kind = "contactor"
poles = 3
load = "M1"
[[device]]
id = "M1"
kind = "motor"
"#;

#[test]
fn the_neutral_passes_a_three_pole_breaker_without_a_break() {
    let (doc, _) = build_document(&pump(THREE_POLE), &library(), Lang::Ru).unwrap();
    let phase = bus_points(&doc, 0);
    let neutral = bus_points(&doc, 3);
    // The phase rises from the breaker, the neutral from the entry below it.
    assert!(neutral[0].1 > phase[0].1);
}

#[test]
fn a_supply_line_can_be_moved_up_and_down() {
    let mut project = pump(THREE_POLE);
    let (doc, _) = build_document(&project, &library(), Lang::Ru).unwrap();
    let (before, other) = (bus_points(&doc, 0), bus_points(&doc, 1));
    project
        .panel_layout
        .set_shift(&crab_core::bus_key(0, 0), 0.0, 20.0);
    let (doc, _) = build_document(&project, &library(), Lang::Ru).unwrap();
    let (after, other_after) = (bus_points(&doc, 0), bus_points(&doc, 1));
    assert_eq!(after[1].1, before[1].1 + 20.0);
    assert_eq!(other_after[1].1, other[1].1);
    project.panel_layout.prune(&project.panel);
    assert_eq!(
        project.panel_layout.shift(&crab_core::bus_key(0, 0)).1,
        20.0
    );
}

fn with_links(links: &str) -> Project {
    pump(&format!("{PUMP_PANEL}\n{links}"))
}

#[test]
fn a_wire_drawn_by_hand_replaces_the_automatic_one_at_its_terminals() {
    let project = with_links("[[link]]\nfrom = \"K1:A2\"\nto = \"B1:-\"\n");
    let nets = crab_schematic::build_nets(&project, &library());
    // The coil's A2 goes to the sensor now, not to the 0 V line.
    assert!(nets.wires.iter().any(|w| w.key() == "K1:A2>B1:-"));
    assert!(nets
        .wires
        .iter()
        .all(|w| w.from.device != "K1" || w.from.terminal != "A2" || w.to.device == "B1"));
    assert!(nets.manual.contains(&"K1:A2>B1:-".to_owned()));
    // A link to a device that is not there is left out.
    let project = with_links("[[link]]\nfrom = \"K1:A2\"\nto = \"ZZ:1\"\n");
    let nets = crab_schematic::build_nets(&project, &library());
    assert!(nets.manual.is_empty());
    assert!(nets
        .wires
        .iter()
        .any(|w| w.from.terminal == "A2" && w.to.terminal == "0V"));
}

#[test]
fn a_device_put_on_the_control_sheet_is_drawn_with_its_terminals_and_wires() {
    use crab_schematic::ItemKind;
    let mut project = with_links("[[link]]\nfrom = \"Q2:2\"\nto = \"K1:A2\"\n");
    project.panel_layout.place("Q2", 0, 300.0, 300.0);
    let (doc, _) = build_document(&project, &library(), Lang::Ru).unwrap();
    let sheet = doc
        .sheets
        .iter()
        .find(|s| {
            s.items
                .iter()
                .any(|i| matches!(&i.kind, ItemKind::Symbol { view: "board", .. }))
        })
        .expect("control sheet");
    let ports = sheet
        .items
        .iter()
        .find_map(|i| match &i.kind {
            ItemKind::Symbol {
                view: "control",
                device,
                ports,
                ..
            } if device == "Q2" => Some(ports.len()),
            _ => None,
        })
        .expect("Q2 on the control sheet");
    assert!(ports >= 4);
    assert!(sheet
        .items
        .iter()
        .any(|i| matches!(&i.kind, ItemKind::Wire { key, .. } if key == "Q2:2>K1:A2")));
}

#[test]
fn links_follow_renames_and_go_with_removed_devices() {
    let mut project = with_links("[[link]]\nfrom = \"K1:A2\"\nto = \"B1:-\"\n");
    project.panel.rename("B1", "B7").unwrap();
    assert_eq!(project.panel.links[0].to, "B7:-");
    project.panel.remove("B7");
    assert!(project.panel.links.is_empty());
}

#[test]
fn a_device_put_on_the_power_sheet_is_drawn_there_not_on_the_control_sheet() {
    use crab_schematic::ItemKind;
    // A free device (not connected) put on the power sheet by hand is drawn there.
    let mut project = with_links("");
    project
        .panel
        .devices
        .push(toml::from_str("id = \"Q9\"\nkind = \"breaker\"\nfree = true").unwrap());
    project.panel_layout.place_power("Q9", 0, 300.0, 300.0);
    let (doc, _) = build_document(&project, &library(), Lang::Ru).unwrap();
    let has_q9 = |title: &str| {
        doc.sheets
            .iter()
            .filter(|s| s.title.starts_with(title))
            .any(|s| {
                s.items.iter().any(|i| {
                    matches!(&i.kind, ItemKind::Symbol { view: "control", device, .. } if device == "Q9")
                })
            })
    };
    assert!(has_q9("Силовая часть"));
    assert!(!has_q9("Цепи управления"));
    // Q2 is in an automatic chain (it has a feeder): putting it by hand draws no second, manual
    // symbol of Q2 on the power sheet.
    let mut project = with_links("");
    project.panel_layout.place_power("Q2", 0, 300.0, 300.0);
    let (doc, _) = build_document(&project, &library(), Lang::Ru).unwrap();
    let manual_q2 = doc
        .sheets
        .iter()
        .filter(|s| s.title.starts_with("Силовая часть"))
        .flat_map(|s| &s.items)
        .any(|i| {
            matches!(&i.kind, ItemKind::Symbol { view: "control", device, .. } if device == "Q2")
        });
    assert!(!manual_q2);
}

#[test]
fn a_coil_of_a_free_contactor_put_on_the_power_sheet_is_its_own_symbol() {
    use crab_schematic::ItemKind;
    let mut project = with_links("");
    project
        .panel
        .devices
        .push(toml::from_str("id = \"K9\"\nkind = \"contactor\"\nfree = true").unwrap());
    project.panel_layout.place_power("K9/coil", 0, 300.0, 300.0);
    let (doc, _) = build_document(&project, &library(), Lang::Ru).unwrap();
    let drawn = doc
        .sheets
        .iter()
        .filter(|s| s.title.starts_with("Силовая часть"))
        .flat_map(|s| &s.items)
        .any(|i| {
            matches!(&i.kind, ItemKind::Symbol { view: "control", device, .. } if device == "K9/coil")
        });
    assert!(drawn);
}

/// A coil of a free contactor put on the power sheet by hand is the place of the contactor's coil:
/// the references of the other sheets point to that sheet.
#[test]
fn a_coil_put_by_hand_on_the_power_sheet_is_the_place_of_its_contactor() {
    let mut project = with_links("");
    project
        .panel
        .devices
        .push(toml::from_str("id = \"K9\"\nkind = \"contactor\"\nfree = true").unwrap());
    project.panel_layout.place_power("K9/coil", 0, 300.0, 300.0);
    let (doc, _) = build_document(&project, &library(), Lang::Ru).unwrap();
    let coil = &doc.coil_at["K9"];
    assert!(doc.sheets[coil.sheet - 1]
        .title
        .starts_with("Силовая часть"));
}
