//! Auxiliary contacts and terminal blocks in the chain (`specs/panel-aux`, task 2).

use std::path::PathBuf;

use crab_core::panel::is_valid_designation;
use crab_core::{check_panel, Library, Panel, Project, Severity};

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
fn a_contact_without_a_coil_is_a_warning_not_an_error() {
    let mut p = panel(BASE);
    p.devices.iter_mut().find(|d| d.id == "Q1_1").unwrap().of = String::new();
    let (lib, _) = Library::load(repo_path("library")).unwrap();
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = p;
    let report = check_panel(&project, &lib);
    let warnings: Vec<&str> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        warnings.contains(&"-Q1_1: контакт не привязан к катушке"),
        "{warnings:#?}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.message.contains("Q1_1") && d.message.contains("аппарата")),
        "{report:#?}"
    );
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

/// A relay whose contacts switch loads of their own (`specs/panel-relay-contacts`, task 3).
const RELAY: &str = r#"
[[device]]
id = "Q1"
kind = "breaker"
load = "K2_1"

[[device]]
id = "K2"
kind = "relay"

[[device]]
id = "K2_1"
kind = "aux"
of = "K2"
load = "E1"

[[device]]
id = "K2_2"
kind = "aux"
of = "K2"
form = "changeover"
from = "Q1"
load = "E2"

[[device]]
id = "E1"
kind = "load"

[[device]]
id = "E2"
kind = "load"

[[device]]
id = "E3"
kind = "load"
from = "K2_1"
"#;

#[test]
fn a_relay_contact_switches_a_load_of_its_own() {
    assert_eq!(errors(&panel(RELAY)), Vec::<String>::new());
}

#[test]
fn only_a_relay_contact_has_a_load_and_not_together_with_a_circuit() {
    // A contact of a breaker or a contactor does not switch a load of its own.
    let mut p = panel(BASE);
    p.devices.iter_mut().find(|d| d.id == "Q1_1").unwrap().load = "M1".into();
    assert!(errors(&p)
        .iter()
        .any(|m| m.contains("свою нагрузку коммутирует только контакт реле")));

    // A relay contact either switches a load or stands in a control circuit.
    let mut p = panel(RELAY);
    p.devices
        .iter_mut()
        .find(|d| d.id == "K2_1")
        .unwrap()
        .series = "K2".into();
    assert!(errors(&p)
        .iter()
        .any(|m| m.contains("K2_1") && m.contains("нужно что-то одно")));

    // An old relay that switches the same load by its poles.
    let mut p = panel(RELAY);
    p.devices.iter_mut().find(|d| d.id == "K2").unwrap().load = "E1".into();
    assert!(errors(&p)
        .iter()
        .any(|m| m.contains("уже коммутирует само реле -K2")));
}

#[test]
fn a_relay_contact_is_fed_and_feeds_a_load_but_a_breaker_contact_does_not() {
    let mut p = panel(RELAY);
    // «Питается от» of the load names the contact: the contact gets the load.
    p.reconnect_power("E2", Some("K2_1")).unwrap();
    assert_eq!(p.device("K2_1").unwrap().load, "E2");
    // The contact itself is fed by the breaker.
    p.reconnect_power("K2_1", Some("Q1")).unwrap();
    assert_eq!(p.device("Q1").unwrap().load, "K2_1");
    // A branch from the contact.
    p.branch_power("E3", Some("K2_1")).unwrap();
    assert_eq!(p.device("E3").unwrap().from, "K2_1");

    // A contact of a breaker stays out of the chain.
    let mut q = panel(BASE);
    assert!(q.reconnect_power("Q1_1", Some("Q1")).is_err());
    assert!(q.reconnect_power("M1", Some("Q1_1")).is_err());
}

#[test]
fn feeding_from_a_busy_source_makes_a_branch_and_back_to_the_load() {
    let mut p = panel(RELAY);
    // Q1 feeds K2_1 already: E2 gets the line of Q1 by a branch, K2_1 stays.
    p.feed_from("E2", Some("Q1")).unwrap();
    assert_eq!(p.device("Q1").unwrap().load, "K2_1");
    assert_eq!(p.device("E2").unwrap().from, "Q1");
    // A free source takes the device as its load; the branch is let go.
    p.feed_from("E2", Some("K2_2")).unwrap();
    assert_eq!(p.device("K2_2").unwrap().load, "E2");
    assert_eq!(p.device("E2").unwrap().from, "");
    // Straight from the feed: nothing feeds it.
    p.feed_from("E2", None).unwrap();
    assert!(p.devices.iter().all(|d| d.load != "E2"));
    assert_eq!(p.device("E2").unwrap().from, "");
    // A refusal changes nothing.
    let before = p.clone();
    assert!(p.feed_from("Q1", Some("Q1")).is_err());
    assert_eq!(p, before);
}
