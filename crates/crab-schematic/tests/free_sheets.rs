//! Free sheets and the names of the drawing sheets (`specs/panel-free-sheets`): a device put by hand
//! keeps its sheet name (`power:0`, `control:0`, `free:0`), so the numbers of the other sheets do
//! not move it.

use std::path::PathBuf;

use crab_core::{Library, PanelLayout, Project, SheetId};
use crab_schematic::{build_document, Document, ItemKind, Lang, Sheet};

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

/// A feed with a breaker and a contactor bound to the board, and its motor. The free devices are
/// not connected to anything, so they are not in a power chain.
const PANEL: &str = r#"
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
bind = "pump"
load = "M1"

[[device]]
id = "M1"
kind = "motor"

[[device]]
id = "Q8"
kind = "breaker"
free = true

[[device]]
id = "Q9"
kind = "breaker"
free = true

[[device]]
id = "K9"
kind = "contactor"
free = true
"#;

fn pump(panel: &str) -> Project {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str(panel).unwrap();
    project
}

fn document(project: &Project) -> Document {
    build_document(project, &library(), Lang::Ru).unwrap().0
}

/// The number of the power sheets and of the control sheets of a document.
fn counts(doc: &Document) -> (usize, usize) {
    let power = doc
        .sheets
        .iter()
        .filter(|s| s.title.starts_with("Силовая часть"))
        .count();
    let control = doc
        .sheets
        .iter()
        .filter(|s| matches!(s.id, Some(SheetId::Control(_))))
        .count();
    (power, control)
}

/// The sheet with the document number `number` (the title sheet is 1).
fn sheet(doc: &Document, number: usize) -> &Sheet {
    &doc.sheets[number - 1]
}

/// The document number of the first free sheet: after the title, the power and the control sheets.
fn first_free(doc: &Document) -> usize {
    let (power, control) = counts(doc);
    2 + power + control
}

fn has_symbol(sheet: &Sheet, view: &str, device: &str) -> bool {
    sheet.items.iter().any(|i| {
        matches!(&i.kind, ItemKind::Symbol { view: v, device: d, .. } if *v == view && d == device)
    })
}

fn has_wire(sheet: &Sheet, key: &str) -> bool {
    sheet
        .items
        .iter()
        .any(|i| matches!(&i.kind, ItemKind::Wire { key: k, .. } if k == key))
}

/// A layout of the old file form: a breaker on a power sheet (`power = true`) and a breaker on a
/// control sheet (`power = false`), both numbered inside their group.
fn old_layout() -> PanelLayout {
    toml::from_str(
        "[placed.Q9]\npower = true\nsheet = 0\nx = 300.0\ny = 300.0\n\n\
         [placed.Q8]\nsheet = 0\nx = 500.0\ny = 300.0\n",
    )
    .unwrap()
}

/// Branches of the feed `X1`, each a breaker with its motor: enough to need a second power sheet.
fn with_branches(panel: &str) -> String {
    let mut text = panel.to_owned();
    for i in 1..=12 {
        text.push_str(&format!(
            "[[device]]\nid = \"B{i}\"\nkind = \"breaker\"\nload = \"N{i}\"\n\
             [[device]]\nid = \"N{i}\"\nkind = \"motor\"\n"
        ));
    }
    text
}

#[test]
fn an_old_file_form_reads_as_power_and_control_sheet_names() {
    let mut project = pump(PANEL);
    project.panel_layout = old_layout();
    assert_eq!(project.panel_layout.placed["Q9"].sheet, SheetId::Power(0));
    assert_eq!(project.panel_layout.placed["Q8"].sheet, SheetId::Control(0));
    // The same layout written with the names draws the same sheets.
    let old = document(&project);
    let text = toml::to_string(&project.panel_layout).unwrap();
    assert!(text.contains("sheet = \"power:0\""), "{text}");
    assert!(text.contains("sheet = \"control:0\""), "{text}");
    assert!(!text.contains("power = "), "{text}");
    project.panel_layout = toml::from_str(&text).unwrap();
    let new = document(&project);
    assert_eq!(old.sheets.len(), new.sheets.len());
    for (a, b) in old.sheets.iter().zip(&new.sheets) {
        assert_eq!(a.title, b.title);
        assert_eq!(a.body, b.body, "{}", a.title);
    }
}

#[test]
fn drawing_sheets_carry_their_name_and_the_others_none() {
    let project = pump(PANEL);
    let doc = document(&project);
    assert_eq!(doc.sheets[0].id, None, "title sheet");
    let (power, control) = counts(&doc);
    let mut expected: Vec<SheetId> = (0..power).map(SheetId::Power).collect();
    expected.extend((0..control).map(SheetId::Control));
    let drawn: Vec<SheetId> = doc.sheets.iter().filter_map(|s| s.id).collect();
    assert_eq!(drawn, expected);
    // The tables come after the drawing sheets and have no name.
    assert!(doc.sheets.len() > 1 + drawn.len());
    assert!(doc.sheets[1 + drawn.len()..].iter().all(|s| s.id.is_none()));
}

#[test]
fn a_caption_names_the_sheet_and_the_title_sheet_lists_it() {
    let mut project = pump(PANEL);
    project.panel_layout.set_caption(SheetId::Power(0), "Ввод");
    let doc = document(&project);
    assert_eq!(sheet(&doc, 2).title, "Ввод");
    assert!(sheet(&doc, 1).body.contains("Ввод"));
    // The other sheets keep their own titles.
    // The control sheet of K1 serves its motor M1.
    assert_eq!(sheet(&doc, 3).title, "Управление -M1");
}

#[test]
fn a_device_put_on_a_free_sheet_is_drawn_there_with_the_blank_sheet_before_it() {
    let panel = format!("{PANEL}\n[[link]]\nfrom = \"Q8:2\"\nto = \"Q9:2\"\n");
    let mut project = pump(&panel);
    project
        .panel_layout
        .place("Q8", SheetId::Free(1), 300.0, 300.0);
    project
        .panel_layout
        .place("Q9", SheetId::Free(1), 500.0, 300.0);
    let doc = document(&project);
    let free: Vec<SheetId> = doc
        .sheets
        .iter()
        .filter_map(|s| s.id)
        .filter(|id| matches!(id, SheetId::Free(_)))
        .collect();
    assert_eq!(free, vec![SheetId::Free(0), SheetId::Free(1)]);
    // The sheet before has nothing drawn; the devices and the wire are on `free:1`.
    let first = first_free(&doc);
    let blank = sheet(&doc, first);
    assert!(blank.items.is_empty());
    assert_eq!(blank.title, "Схема 1");
    let here = sheet(&doc, first + 1);
    assert_eq!(here.id, Some(SheetId::Free(1)));
    assert_eq!(here.title, "Схема 2");
    assert!(has_symbol(here, "control", "Q8"));
    assert!(has_symbol(here, "control", "Q9"));
    assert!(has_wire(here, "Q8:2>Q9:2"));
}

#[test]
fn a_coil_put_on_a_free_sheet_is_the_place_of_its_contactor() {
    let mut project = pump(PANEL);
    project
        .panel_layout
        .place("K9/coil", SheetId::Free(0), 300.0, 300.0);
    let doc = document(&project);
    let first = first_free(&doc);
    assert_eq!(doc.coil_at["K9"].sheet, first);
    assert!(has_symbol(sheet(&doc, first), "control", "K9/coil"));
}

#[test]
fn a_device_of_a_power_chain_put_by_hand_is_not_drawn_again() {
    let mut project = pump(PANEL);
    // K1 stands in the power chain (and is bound to the board): the generator draws it, so a hand
    // copy on a free sheet is not drawn and the free sheet stays blank.
    project
        .panel_layout
        .place("K1", SheetId::Free(0), 300.0, 300.0);
    let doc = document(&project);
    let free = sheet(&doc, first_free(&doc));
    assert_eq!(free.id, Some(SheetId::Free(0)));
    assert!(free.items.is_empty());
    assert!(doc.sheets.iter().any(|s| has_symbol(s, "power", "K1")));
}

#[test]
fn a_panel_without_a_bound_terminal_puts_its_hand_items_on_free_sheets() {
    // No terminal of the board is bound: there is no control sheet, and the hand items go to the
    // free sheets after the power sheets.
    let unbound = PANEL.replace("bind = \"pump\"\n", "");
    let (power, control) = counts(&document(&pump(&unbound)));
    assert_eq!(control, 0);
    let mut project = pump(&unbound);
    project
        .panel_layout
        .place("Q8", SheetId::Free(0), 300.0, 300.0);
    let doc = document(&project);
    let here = sheet(&doc, 2 + power);
    assert_eq!(here.id, Some(SheetId::Free(0)));
    assert!(has_symbol(here, "control", "Q8"));
    assert_eq!(here.title, "Схема 1");
}

/// The regression this naming fixes: a hand item on the first control sheet stays there when a new
/// motor adds power sheets, and a caption stays with its sheet.
#[test]
fn a_control_item_stays_on_its_sheet_when_a_power_sheet_is_added() {
    let mut project = pump(PANEL);
    project
        .panel_layout
        .place("Q8", SheetId::Control(0), 500.0, 300.0);
    project
        .panel_layout
        .set_caption(SheetId::Control(0), "Пуск насоса");
    let before = document(&project);
    let (power_before, _) = counts(&before);

    project.panel = toml::from_str(&with_branches(PANEL)).unwrap();
    let after = document(&project);
    let (power_after, _) = counts(&after);
    assert!(power_after > power_before, "{power_after} > {power_before}");

    let control = after
        .sheets
        .iter()
        .position(|s| s.id == Some(SheetId::Control(0)))
        .unwrap();
    assert_eq!(after.sheets[control].title, "Пуск насоса");
    assert!(has_symbol(&after.sheets[control], "control", "Q8"));
    let drawn = after
        .sheets
        .iter()
        .filter(|s| has_symbol(s, "control", "Q8"))
        .count();
    assert_eq!(drawn, 1);
}

#[test]
fn a_hand_item_past_the_last_sheet_of_its_kind_is_drawn_on_the_last_one() {
    let mut project = pump(PANEL);
    project
        .panel_layout
        .place("Q8", SheetId::Control(7), 500.0, 300.0);
    project
        .panel_layout
        .place("Q9", SheetId::Power(7), 300.0, 300.0);
    let doc = document(&project);
    let (power, control) = counts(&doc);
    assert_eq!((power, control), (1, 1));
    assert!(has_symbol(sheet(&doc, 2 + power), "control", "Q8"));
    assert!(has_symbol(sheet(&doc, 2), "control", "Q9"));
}

#[test]
fn a_control_sheet_with_hand_items_only_is_made_without_a_terminal_of_the_board() {
    let unbound = PANEL.replace("bind = \"pump\"\n", "");
    let mut project = pump(&unbound);
    project
        .panel_layout
        .place("Q8", SheetId::Control(0), 500.0, 300.0);
    let doc = document(&project);
    let (power, control) = counts(&doc);
    assert_eq!(control, 1);
    assert!(has_symbol(sheet(&doc, 2 + power), "control", "Q8"));
}

#[test]
fn removing_a_free_sheet_moves_the_later_ones_with_their_items_and_captions() {
    let mut project = pump(PANEL);
    project
        .panel_layout
        .place("Q8", SheetId::Free(0), 300.0, 300.0);
    project
        .panel_layout
        .place("Q9", SheetId::Free(2), 300.0, 300.0);
    project.panel_layout.set_caption(SheetId::Free(2), "Вторая");
    project.panel_layout.remove_free_sheet(0);
    let doc = document(&project);
    let first = first_free(&doc);
    assert_eq!(counts(&doc).1, 1, "the control sheet stays");
    assert!(!doc.sheets.iter().any(|s| has_symbol(s, "control", "Q8")));
    assert!(sheet(&doc, first).items.is_empty());
    let here = sheet(&doc, first + 1);
    assert_eq!(here.id, Some(SheetId::Free(1)));
    assert_eq!(here.title, "Вторая");
    assert!(has_symbol(here, "control", "Q9"));
}

#[test]
fn a_caption_on_an_empty_free_sheet_adds_that_sheet() {
    let mut project = pump(PANEL);
    project.panel_layout.set_caption(SheetId::Free(1), "Резерв");
    let doc = document(&project);
    let first = first_free(&doc);
    assert_eq!(sheet(&doc, first).title, "Схема 1");
    assert_eq!(sheet(&doc, first + 1).title, "Резерв");
    assert!(sheet(&doc, first + 1).items.is_empty());
    let free = doc
        .sheets
        .iter()
        .filter(|s| matches!(s.id, Some(SheetId::Free(_))))
        .count();
    assert_eq!(free, 2);
}

#[test]
fn an_empty_layout_draws_no_free_sheet() {
    let doc = document(&pump(PANEL));
    assert!(!doc
        .sheets
        .iter()
        .any(|s| matches!(s.id, Some(SheetId::Free(_)))));
}
