//! LoRa E220: checks of the modem and its partner blocks.
use std::path::PathBuf;

use crab_core::{check, DiagKind, Library, Logic, Report};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn library() -> Library {
    let (lib, errors) = Library::load(root().join("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    lib
}

fn run(src: &str) -> Report {
    let logic: Logic = toml::from_str(src).unwrap();
    check(&logic, &library(), None)
}

fn lora_errors(report: &Report) -> Vec<String> {
    report
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagKind::Lora)
        .map(|d| d.message.clone())
        .collect()
}

const PINS: &str = "tx = 17\nrx = 16\nm0 = 25\nm1 = 27\naux = 33\n";

fn centre(extra: &str) -> String {
    format!("[[block]]\nid = \"m\"\ntype = \"lora_e220\"\nrole = 0\n{PINS}\n{extra}")
}

fn point(id: &str, modem: &str, node: i64) -> String {
    format!(
        "[[block]]\nid = \"{id}\"\ntype = \"lora_point\"\nmodem = \"{modem}\"\nnode = {node}\nn_cmd = 0\nn_ans = 1\nt_ans0 = 1\n\n"
    )
}

#[test]
fn a_centre_with_two_slaves_is_fine() {
    let src = centre(&(point("a", "m", 0) + &point("b", "m", 1)));
    assert_eq!(lora_errors(&run(&src)), Vec::<String>::new());
}

#[test]
fn the_modem_needs_its_pins_but_not_aux_or_m1() {
    let src = format!(
        "[[block]]\nid = \"m\"\ntype = \"lora_e220\"\ntx = 17\nrx = 16\n\n{}",
        point("a", "m", 0)
    );
    let errors = lora_errors(&run(&src));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("M0"));
}

#[test]
fn a_centre_without_slaves_is_an_error() {
    assert!(lora_errors(&run(&centre("")))[0].contains("нет ни одного"));
}

#[test]
fn slave_addresses_must_differ() {
    let src = centre(&(point("a", "m", 2) + &point("b", "m", 2)));
    let errors = lora_errors(&run(&src));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("адрес слэйва 2 уже занят"));
}

#[test]
fn at_most_eight_slaves() {
    let many: String = (0..9).map(|n| point(&format!("s{n}"), "m", n)).collect();
    let errors = lora_errors(&run(&centre(&many)));
    assert!(
        errors.iter().any(|e| e.contains("при пределе 8")),
        "{errors:?}"
    );
}

#[test]
fn the_partner_must_name_an_existing_modem() {
    for (modem, text) in [
        ("", "не выбран модем"),
        ("nope", "нет в проекте"),
        ("a", "не модем"),
    ] {
        let src = centre(&point("a", modem, 0));
        let errors = lora_errors(&run(&src));
        assert!(
            errors.iter().any(|e| e.contains(text)),
            "{modem}: {errors:?}"
        );
    }
}

#[test]
fn the_role_of_the_modem_must_fit_the_block() {
    // lora_centre on a centre modem, and lora_point on a slave modem.
    let wrong =
        "[[block]]\nid = \"c\"\ntype = \"lora_centre\"\nmodem = \"m\"\nn_cmd = 0\nn_ans = 0\n";
    let errors = lora_errors(&run(&centre(wrong)));
    assert!(
        errors
            .iter()
            .any(|e| e.contains("нужен модем с ролью «слэйв»")),
        "{errors:?}"
    );
    let slave = format!(
        "[[block]]\nid = \"m\"\ntype = \"lora_e220\"\nrole = 1\n{PINS}\n{}",
        point("a", "m", 0)
    );
    let errors = lora_errors(&run(&slave));
    assert!(
        errors
            .iter()
            .any(|e| e.contains("нужен модем с ролью «мастер»")),
        "{errors:?}"
    );
}

#[test]
fn a_slave_needs_exactly_one_centre_block() {
    let modem = format!("[[block]]\nid = \"m\"\ntype = \"lora_e220\"\nrole = 1\n{PINS}\n");
    assert!(lora_errors(&run(&modem))[0].contains("нет блока"));
    let one =
        "[[block]]\nid = \"c1\"\ntype = \"lora_centre\"\nmodem = \"m\"\nn_cmd = 0\nn_ans = 0\n\n";
    assert!(lora_errors(&run(&format!("{modem}{one}"))).is_empty());
    let two = one.replace("c1", "c2");
    let errors = lora_errors(&run(&format!("{modem}{one}{two}")));
    assert!(
        errors.iter().any(|e| e.contains("должен быть один")),
        "{errors:?}"
    );
}

#[test]
fn the_examples_are_clean() {
    for ex in ["examples/lora_star", "examples/lora_slave"] {
        let project = crab_core::Project::load(root().join(ex)).unwrap();
        let report = check(&project.logic, &library(), None);
        assert!(lora_errors(&report).is_empty(), "{ex}");
    }
}

#[test]
fn the_pins_of_the_modem_may_not_coincide() {
    let src = "[[block]]\nid = \"m\"\ntype = \"lora_e220\"\ntx = 17\nrx = 17\nm0 = 25\nm1 = 27\n\n"
        .to_owned()
        + &point("a", "m", 0);
    let report = run(&src);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.kind == DiagKind::PinBusy),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn m1_may_be_tied_to_m0() {
    let src = "[[block]]\nid = \"m\"\ntype = \"lora_e220\"\ntx = 17\nrx = 16\nm0 = 25\nm1 = -1\n\n"
        .to_owned()
        + &point("a", "m", 0);
    assert_eq!(lora_errors(&run(&src)), Vec::<String>::new());
}

#[test]
fn the_modem_of_a_partner_is_chosen_from_a_list_of_blocks() {
    let lib = library();
    for kind in ["lora_point", "lora_centre"] {
        let def = &lib.get(kind).unwrap().def;
        let p = def.param("modem").unwrap();
        assert_eq!(p.of_block, "lora_e220", "{kind}");
    }
}

#[test]
fn the_modem_is_a_device_not_a_tag_and_is_deleted_with_its_block() {
    let lib = library();
    let mut project = crab_core::Project::load(root().join("examples/lora_star")).unwrap();
    assert!(!project.is_tag(&lib, "modem"));
    project.remove_block("modem").unwrap();
    assert!(project.logic.block("modem").is_none());
    assert!(!project.layout.blocks.contains_key("modem"));
}
