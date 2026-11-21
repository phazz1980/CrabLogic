//! Sheet formats and the layout inside the frame (`specs/electrical-schematics`, task 7a).

use std::path::PathBuf;

use crab_core::{Library, Panel, Project};
use crab_schematic::{build_document_in, source_in, Format, Lang, Page};

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

const PANEL: &str = r#"
supply = "ac400"

[[device]]
id = "X1"
kind = "feed"
rating = "400 В, 25 А"
[[device]]
id = "Q1"
kind = "breaker"
rating = "C16"
load = "K1"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
rating = "0,75 кВт"
[[device]]
id = "Q2"
kind = "breaker"
rating = "C10"
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

fn project(panel: &str) -> Project {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(panel).unwrap();
    project
}

/// The points a drawing call puts on the sheet (its anchor; lines give every vertex).
fn points(line: &str) -> Vec<(f32, f32)> {
    let num = |s: &str| s.trim().parse::<f32>().ok();
    if let Some(args) = line.strip_prefix("W(").or_else(|| line.strip_prefix("WD(")) {
        // W("#rrggbb", width, (x, y), (x, y)...)
        return args
            .split('(')
            .skip(1)
            .filter_map(|p| {
                let p = p.trim_end_matches([')', ',', ' ']);
                let (x, y) = p.split_once(',')?;
                Some((num(x)?, num(y)?))
            })
            .collect();
    }
    // Text: the extent of the drawn text (the font is about 0.45 of its size wide per character),
    // not the box the markup puts it in.
    for call in ["T(", "TB("] {
        if let Some(args) = line.strip_prefix(call) {
            let parts: Vec<&str> = args.splitn(7, ", ").collect();
            if let [x, _, w, al, size, _, text] = parts[..] {
                let (Some(x), Some(w), Some(size)) = (num(x), num(w), num(size)) else {
                    continue;
                };
                let y = num(parts[1]).unwrap_or_default();
                let width = text.trim_end_matches(')').chars().count() as f32 * 0.45 * size;
                let (left, right) = match al {
                    "left" => (x, x + width),
                    "center" => (x + w / 2.0 - width / 2.0, x + w / 2.0 + width / 2.0),
                    _ => (x + w - width, x + w),
                };
                return vec![(left, y), (right, y)];
            }
        }
    }
    for call in ["RO(", "CO(", "D(", "TV("] {
        if let Some(args) = line.strip_prefix(call) {
            let mut it = args.split(", ");
            if let (Some(x), Some(y)) = (it.next().and_then(num), it.next().and_then(num)) {
                return vec![(x, y)];
            }
        }
    }
    Vec::new()
}

#[test]
fn every_call_of_a_body_lies_inside_the_frame_on_both_formats() {
    let lib = library();
    for (format, lang) in [
        (Format::A3, Lang::Ru),
        (Format::A4, Lang::Ru),
        (Format::A3, Lang::En),
        (Format::A4, Lang::En),
    ] {
        let page = Page::new(format);
        let (doc, _) = build_document_in(&project(PANEL), &lib, lang, format).unwrap();
        assert!(doc.sheets.len() >= 6);
        for sheet in &doc.sheets {
            for line in sheet.body.lines() {
                for (x, y) in points(line) {
                    // A little room: the anchor of a text is its left edge or its box.
                    assert!(
                        x >= page.x0() - 40.0 && x <= page.x1() + 10.0,
                        "{format:?} «{}»: x = {x} в {line}",
                        sheet.title
                    );
                    assert!(
                        y >= page.y0() - 10.0 && y <= page.y1() + 30.0,
                        "{format:?} «{}»: y = {y} в {line}",
                        sheet.title
                    );
                }
            }
        }
    }
}

#[test]
fn a4_is_asked_for_in_the_paper_and_has_more_sheets_than_a3() {
    let lib = library();
    let a3 = source_in(&project(PANEL), &lib, Lang::Ru, Format::A3).unwrap();
    let a4 = source_in(&project(PANEL), &lib, Lang::Ru, Format::A4).unwrap();
    assert!(a3.contains("paper: \"a3\""));
    assert!(a4.contains("paper: \"a4\""));
    let n3 = build_document_in(&project(PANEL), &lib, Lang::Ru, Format::A3)
        .unwrap()
        .0
        .sheets
        .len();
    let n4 = build_document_in(&project(PANEL), &lib, Lang::Ru, Format::A4)
        .unwrap()
        .0
        .sheets
        .len();
    assert!(n4 >= n3, "{n4} < {n3}");
    // The format is written in the title block.
    assert!(a3.contains("\"A3\"") && a4.contains("\"A4\""));
}

#[test]
fn the_default_format_is_a3() {
    assert_eq!(Format::default(), Format::A3);
    assert_eq!(Format::from_code("A4"), Some(Format::A4));
    assert_eq!(Format::from_code("a5"), None);
}

#[test]
fn references_on_the_same_sheet_leave_the_sheet_out() {
    use crab_schematic::Xref;
    let at = Xref {
        sheet: 3,
        zone: "B2".into(),
    };
    assert_eq!(at.to_string(), "/3.B2");
    assert_eq!(at.from_sheet(3), "/.B2");
    assert_eq!(at.from_sheet(2), "/3.B2");
}

#[test]
fn direct_current_buses_are_l_plus_and_m() {
    let panel = r#"
supply = "dc24"
[[device]]
id = "X1"
kind = "feed"
[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"
[[device]]
id = "M1"
kind = "motor"
"#;
    let lib = library();
    let (doc, _) = build_document_in(&project(panel), &lib, Lang::Ru, Format::A3).unwrap();
    let power = doc
        .sheets
        .iter()
        .find(|s| s.title.starts_with("Силовая часть"))
        .expect("силовой лист");
    assert!(power.body.contains("\"L+\""), "{}", power.title);
    assert!(power.body.contains("\"M\""));
    assert!(!power.body.contains("\"+24V\""));
}
