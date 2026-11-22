//! The drawing as operations (`specs/electrical-schematics`, task 9a): what the editor shows is
//! what goes into the PDF.

use std::path::PathBuf;

use crab_core::{Library, Panel, Project};
use crab_schematic::{build_document_in, Format, Lang, Op};

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

const PANEL: &str = r#"
supply = "ac230"
[[device]]
id = "X1"
kind = "feed"
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
[[device]]
id = "B1"
kind = "sensor"
bind = "sensor.out"
"#;

fn document(format: Format) -> crab_schematic::Document {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty());
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(PANEL).unwrap();
    build_document_in(&project, &lib, Lang::Ru, format)
        .unwrap()
        .0
}

/// How many calls of each kind the Typst markup holds, and the texts it writes.
fn calls(markup: &str) -> (usize, usize, usize, usize, Vec<String>) {
    let (mut lines, mut rects, mut circles, mut dots) = (0, 0, 0, 0);
    let mut texts = Vec::new();
    for line in markup.lines() {
        if line.starts_with("W(") || line.starts_with("WD(") {
            lines += 1;
        } else if line.starts_with("RO(") {
            rects += 1;
        } else if line.starts_with("CO(") {
            circles += 1;
        } else if line.starts_with("D(") {
            dots += 1;
        } else if ["T(", "TB(", "TV("].iter().any(|c| line.starts_with(c)) {
            // The text is the last quoted argument.
            let text = line.rsplit_once(", \"").map_or("", |(_, t)| t);
            texts.push(text.trim_end_matches(')').trim_end_matches('"').to_owned());
        }
    }
    (lines, rects, circles, dots, texts)
}

#[test]
fn every_sheet_has_as_many_operations_as_calls_and_the_same_texts() {
    for format in [Format::A3, Format::A4] {
        let doc = document(format);
        assert!(doc.sheets.len() >= 5);
        for sheet in &doc.sheets {
            let (lines, rects, circles, dots, texts) = calls(&sheet.body);
            let count = |f: fn(&Op) -> bool| sheet.ops.iter().filter(|o| f(o)).count();
            assert_eq!(
                count(|o| matches!(o, Op::Line { .. })),
                lines,
                "{}",
                sheet.title
            );
            assert_eq!(
                count(|o| matches!(o, Op::Rect { .. })),
                rects,
                "{}",
                sheet.title
            );
            assert_eq!(
                count(|o| matches!(o, Op::Circle { .. })),
                circles,
                "{}",
                sheet.title
            );
            assert_eq!(
                count(|o| matches!(o, Op::Dot { .. })),
                dots,
                "{}",
                sheet.title
            );
            let drawn: Vec<&str> = sheet
                .ops
                .iter()
                .filter_map(|o| match o {
                    Op::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(drawn.len(), texts.len(), "{}", sheet.title);
            for (a, b) in drawn.iter().zip(&texts) {
                assert_eq!(a.replace('\\', ""), b.replace('\\', ""), "{}", sheet.title);
            }
        }
    }
}

#[test]
fn the_frame_is_operations_too_and_names_the_sheet() {
    let doc = document(Format::A3);
    let frame = doc.frame_ops(1, Lang::Ru);
    assert!(frame.iter().any(|o| matches!(o, Op::Rect { .. })));
    let texts: Vec<&str> = frame
        .iter()
        .filter_map(|o| match o {
            Op::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(texts.contains(&doc.sheets[1].title.as_str()));
    assert!(texts.contains(&format!("2 / {}", doc.sheets.len()).as_str()));
    assert!(texts.contains(&"A3") && texts.contains(&"12"), "{texts:?}");
    // An index outside the document draws nothing.
    assert!(doc.frame_ops(99, Lang::Ru).is_empty());
    let (w, h) = doc.page_size();
    assert!(w > h);
}
