//! Auxiliary contacts and terminal blocks in the chain (`specs/panel-aux`, task 2).

use std::path::PathBuf;

use crab_core::panel::is_valid_designation;
use crab_core::{check_panel, Library, Panel, Project};

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn panel(src: &str) -> Panel {
    toml::from_str(src).unwrap()
}

fn errors(panel: &Panel) -> Vec<String> {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = panel.clone();
    check_panel(&project, &lib)
        .errors()
        .map(|d| d.message.clone())
        // The tags of the pump example are not what this test is about.
        .filter(|m| !m.contains("без устройства в щите") && !m.contains("не привязано"))
        .collect()
}

const BASE: &str = r#"
[[device]]
id = "Q1"
kind = "breaker"
load = "K1"

[[device]]
id = "Q1_1"
kind = "aux"
of = "Q1"
form = "nc"

[[device]]
id = "K1"
kind = "contactor"
load = "M1"

[[device]]
id = "K1_1"
kind = "aux"
of = "K1"
series = "K1"

[[device]]
id = "M1"
kind = "motor"
"#;

#[test]
fn designations_may_have_dots_between_characters() {
    for ok in ["K1", "X1.1", "Q2.1_2", "насос.1"] {
        assert!(is_valid_designation(ok), "{ok}");
    }
    for bad in ["", ".X1", "X1.", "X1..1", "X 1", "X1:1", "X1>1"] {
        assert!(!is_valid_designation(bad), "{bad}");
    }
}

#[test]
fn a_correct_panel_with_contacts_has_no_errors() {
    assert_eq!(errors(&panel(BASE)), Vec::<String>::new());
}

#[test]
fn a_contact_needs_a_switching_device_and_a_coil() {
    let mut p = panel(BASE);
    p.devices.iter_mut().find(|d| d.id == "Q1_1").unwrap().of = "Z9".into();
    assert!(errors(&p).iter().any(|m| m.contains("нет аппарата «Z9»")));

    let mut p = panel(BASE);
    p.devices.iter_mut().find(|d| d.id == "Q1_1").unwrap().of = "M1".into();
    assert!(errors(&p)
        .iter()
        .any(|m| m.contains("не бывает доп. контактов")));

    let mut p = panel(BASE);
    p.devices
        .iter_mut()
        .find(|d| d.id == "K1_1")
        .unwrap()
        .series = "M1".into();
    assert!(errors(&p).iter().any(|m| m.contains("не катушка")));

    let mut p = panel(BASE);
    p.devices
        .iter_mut()
        .find(|d| d.id == "K1_1")
        .unwrap()
        .series = "Z9".into();
    assert!(errors(&p).iter().any(|m| m.contains("нет катушки «Z9»")));

    let mut p = panel(BASE);
    p.devices.iter_mut().find(|d| d.id == "K1_1").unwrap().bind = "pump".into();
    assert!(errors(&p).iter().any(|m| m.contains("нужно что-то одно")));
}

#[test]
fn rename_moves_the_contacts_and_remove_takes_them_away() {
    let mut p = panel(BASE);
    p.rename("K1", "K7").unwrap();
    assert_eq!(p.device("K1_1").unwrap().of, "K7");
    assert_eq!(p.device("K1_1").unwrap().series, "K7");
    assert_eq!(p.aux_of("K7").count(), 1);

    p.remove("K7");
    assert!(
        p.device("K1_1").is_none(),
        "the contact goes with its device"
    );
    assert!(p.device("Q1_1").is_some());

    // A contact in the circuit of a removed coil stays but loses the circuit.
    let mut p = panel(BASE);
    p.devices
        .iter_mut()
        .find(|d| d.id == "Q1_1")
        .unwrap()
        .series = "K1".into();
    p.remove("K1");
    assert_eq!(p.device("Q1_1").unwrap().series, "");
}

#[test]
fn a_terminal_block_stands_between_a_contactor_and_a_motor() {
    let mut p = panel(
        r#"
[[device]]
id = "K1"
kind = "contactor"
load = "M1"
[[device]]
id = "X1.1"
kind = "terminal"
[[device]]
id = "M1"
kind = "motor"
"#,
    );
    p.reconnect_power("X1.1", Some("K1")).unwrap();
    p.reconnect_power("M1", Some("X1.1")).unwrap();
    assert_eq!(p.device("K1").unwrap().load, "X1.1");
    assert_eq!(p.device("X1.1").unwrap().load, "M1");
    // A contact is not in the power chain.
    let mut q = panel(BASE);
    assert!(q.reconnect_power("Q1_1", Some("Q1")).is_err());
}
