//! Main contacts on the coil of another device (`specs/panel-coil-device`): helpers, check,
//! rename and remove.

use std::path::PathBuf;

use crab_core::{check_panel, rename_device, Library, Panel, Project};

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
[[device]]
id = "Q1"
kind = "breaker"

[[device]]
id = "K1"
kind = "contactor"
bind = "pump"
load = "M1"

[[device]]
id = "K2"
kind = "contactor"
of = "K1"
from = "Q1"
load = "M2"

[[device]]
id = "M1"
kind = "motor"

[[device]]
id = "M2"
kind = "motor"

[[device]]
id = "B1"
kind = "sensor"
bind = "sensor.out"
"#;

fn project(src: &str) -> Project {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(src).unwrap();
    project
}

fn errors(project: &Project) -> Vec<String> {
    check_panel(project, &library())
        .errors()
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn coil_helpers() {
    let p = project(PANEL).panel;
    let (k1, k2) = (p.device("K1").unwrap(), p.device("K2").unwrap());
    assert!(p.has_own_coil(k1) && !p.has_own_coil(k2));
    assert!(p.is_borrowed_contactor(k2) && !p.is_borrowed_contactor(k1));
    assert_eq!(p.coil_device(k2).map(|d| d.id.as_str()), Some("K1"));
    assert_eq!(p.coil_device(k1).map(|d| d.id.as_str()), Some("K1"));
    assert_eq!(p.coil_name(k2), "K1");
    assert_eq!(p.coil_name(k1), "K1");
    assert_eq!(
        p.power_contacts_of("K1")
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        ["K2"]
    );
    // A missing target: no coil.
    let p = project(&PANEL.replace("of = \"K1\"", "of = \"K9\"")).panel;
    assert!(p.coil_device(p.device("K2").unwrap()).is_none());
}

#[test]
fn contacts_of_another_coil_need_no_binding_and_pass_the_check() {
    let found = errors(&project(PANEL));
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn contacts_of_another_coil_with_a_binding_are_refused() {
    let found = errors(&project(
        &PANEL.replace("of = \"K1\"\n", "of = \"K1\"\nbind = \"pump\"\n"),
    ));
    assert!(
        found.iter().any(|m| m.contains("силовые контакты -K2")),
        "{found:#?}"
    );
}

#[test]
fn the_target_must_exist_and_be_a_coil() {
    let found = errors(&project(&PANEL.replace("of = \"K1\"", "of = \"K9\"")));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].contains("нет катушки «K9»"));

    let found = errors(&project(&PANEL.replace("of = \"K1\"", "of = \"M1\"")));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].contains("не катушка"));
}

#[test]
fn contacts_cannot_stand_on_themselves_or_on_a_chain() {
    let found = errors(&project(&PANEL.replace("of = \"K1\"", "of = \"K2\"")));
    assert!(
        found.iter().any(|m| m.contains("собственной катушке")),
        "{found:#?}"
    );

    // K3 on K2, K2 on K1: K2 has no coil of its own to stand on.
    let src = format!(
        "{PANEL}\n[[device]]\nid = \"K3\"\nkind = \"contactor\"\nof = \"K2\"\nfrom = \"Q1\"\n"
    );
    let found = errors(&project(&src));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].contains("цепочки и циклы"));

    // A loop: K1 on K2 and K2 on K1.
    let looped = PANEL.replace("bind = \"pump\"\n", "of = \"K2\"\n");
    let found = errors(&project(&looped));
    assert!(
        found.iter().any(|m| m.contains("цепочки и циклы")),
        "{found:#?}"
    );
}

#[test]
fn a_contact_in_series_needs_a_coil_of_its_own() {
    let src = format!(
        "{PANEL}\n[[device]]\nid = \"K1_1\"\nkind = \"aux\"\nof = \"K1\"\nseries = \"K2\"\n"
    );
    let found = errors(&project(&src));
    assert!(found.iter().any(|m| m.contains("не катушка")), "{found:#?}");
}

#[test]
fn renaming_the_coil_follows_in_of() {
    let mut p = project(PANEL);
    rename_device(&mut p, "K1", "K5").unwrap();
    assert_eq!(p.panel.device("K2").unwrap().of, "K5");
}

#[test]
fn removing_the_coil_makes_the_contacts_a_plain_contactor() {
    let mut p = project(PANEL).panel;
    p.remove("K1");
    let k2 = p
        .device("K2")
        .expect("the contacts stay in the power chain");
    assert!(k2.of.is_empty() && k2.load == "M2");
    assert!(p.has_own_coil(k2));
}
