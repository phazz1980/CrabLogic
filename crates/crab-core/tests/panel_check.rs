//! Check of the electrical panel on the pump example (`specs/electrical-schematics`).

use std::path::PathBuf;

use crab_core::{check_panel, check_project, DiagKind, Library, Panel, Project, Severity};

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

/// The pump example (input `sensor`, output `pump`) with a panel built from `src`.
fn pump_with(src: &str) -> Project {
    let mut project = Project::load(repo_path("examples/pump")).unwrap();
    project.panel = toml::from_str::<Panel>(src).unwrap();
    project
}

const GOOD: &str = r#"
[[device]]
id = "Q1"
kind = "breaker"
rating = "C16"

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

fn errors(project: &Project) -> Vec<String> {
    check_panel(project, &library())
        .errors()
        .map(|d| d.message.clone())
        .collect()
}

/// The panel with one change: `from` in the text of [`GOOD`] becomes `to`.
fn changed(from: &str, to: &str) -> Project {
    assert!(GOOD.contains(from), "{from}");
    pump_with(&GOOD.replace(from, to))
}

fn assert_one_error(project: &Project, needle: &str) {
    let found = errors(project);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].contains(needle), "{found:#?}");
}

#[test]
fn good_panel_has_no_diagnostics() {
    let report = check_panel(&pump_with(GOOD), &library());
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
}

#[test]
fn project_without_panel_is_not_checked() {
    let project = Project::load(repo_path("examples/pump")).unwrap();
    assert!(check_panel(&project, &library()).diagnostics.is_empty());
}

#[test]
fn the_schema_check_does_not_see_the_panel() {
    // A broken panel must never stop the firmware build.
    let project = changed("bind = \"pump\"", "bind = \"nowhere\"");
    assert!(!errors(&project).is_empty());
    let report = check_project(&project, &library());
    assert!(
        report.diagnostics.iter().all(|d| d.kind != DiagKind::Panel),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn bad_and_repeated_designations() {
    // The contactor's `load` points at the renamed motor, so it fails too: look only for ours.
    let found = errors(&changed("id = \"M1\"", "id = \"M 1\""));
    assert_eq!(
        found.iter().filter(|m| m.contains("недопустимое")).count(),
        1,
        "{found:#?}"
    );
    let found = errors(&changed("id = \"B1\"", "id = \"Q1\""));
    assert_eq!(
        found
            .iter()
            .filter(|m| m.contains("занято несколькими"))
            .count(),
        1,
        "{found:#?}"
    );
}

#[test]
fn contactor_needs_a_binding() {
    // The output `pump` is left without a device as well.
    let found = errors(&changed("bind = \"pump\"\n", ""));
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(found.iter().any(|m| m.contains("-K1 не привязано")));
    assert!(found
        .iter()
        .any(|m| m.contains("выход «pump» без устройства")));
}

#[test]
fn unknown_binding_is_named() {
    let found = errors(&changed("bind = \"pump\"", "bind = \"nowhere\""));
    assert!(
        found
            .iter()
            .any(|m| m.contains("нет входа или выхода «nowhere»")),
        "{found:#?}"
    );
}

#[test]
fn coil_on_an_input_and_sensor_on_an_output_are_refused() {
    let found = errors(&changed("bind = \"pump\"", "bind = \"sensor\""));
    assert!(
        found
            .iter()
            .any(|m| m.contains("-K1") && m.contains("выходу платы")),
        "{found:#?}"
    );
    let found = errors(&changed("bind = \"sensor.out\"", "bind = \"pump\""));
    assert!(
        found
            .iter()
            .any(|m| m.contains("-B1") && m.contains("входу платы")),
        "{found:#?}"
    );
}

#[test]
fn analog_pin_does_not_fit_a_button() {
    let found = errors(&changed("kind = \"sensor\"", "kind = \"button\""));
    assert!(
        found
            .iter()
            .any(|m| m.contains("дискретному входу") && m.contains("аналоговый")),
        "{found:#?}"
    );
}

#[test]
fn one_output_for_one_device() {
    let extra = format!("{GOOD}\n[[device]]\nid = \"K2\"\nkind = \"relay\"\nbind = \"pump\"\n");
    let found = errors(&pump_with(&extra));
    assert!(
        found
            .iter()
            .any(|m| m.contains("«pump» уже привязан к -K1") && m.contains("-K2")),
        "{found:#?}"
    );
}

#[test]
fn load_must_exist_and_not_be_itself() {
    assert_one_error(
        &changed("load = \"M1\"", "load = \"M9\""),
        "нет устройства «M9»",
    );
    assert_one_error(&changed("load = \"M1\"", "load = \"K1\""), "само себя");
}

#[test]
fn input_and_output_without_a_device_are_reported() {
    let panel = r#"
[[device]]
id = "Q1"
kind = "breaker"
"#;
    let found = errors(&pump_with(panel));
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(found
        .iter()
        .any(|m| m.contains("выход «pump» без устройства")));
    assert!(found
        .iter()
        .any(|m| m.contains("вход «sensor» без устройства")));
}

#[test]
fn wrong_letter_is_a_warning_only() {
    let project = changed("id = \"Q1\"", "id = \"F1\"");
    assert!(errors(&project).is_empty());
    let report = check_panel(&project, &library());
    let warnings: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .collect();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].message.contains("начинается с «Q»"));
}

#[test]
fn a_device_cannot_be_named_as_a_bus() {
    // `bus:<name>` in a link is a supply line: the device name is taken, in any case.
    for name in ["bus", "BUS"] {
        let found = errors(&changed("id = \"M1\"", &format!("id = \"{name}\"")));
        let taken = format!("-{name}: обозначение занято под шины, выберите другое");
        assert_eq!(
            found.iter().filter(|m| m.contains(&taken)).count(),
            1,
            "{found:#?}"
        );
    }
}

#[test]
fn a_coil_wired_by_hand_needs_no_binding() {
    // Unbound, the contactor is refused; with a hand wire to its coil it is driven by that wire.
    let unbound = changed("bind = \"pump\"\n", "");
    assert!(errors(&unbound)
        .iter()
        .any(|m| m.contains("не привязано к входу или выходу проекта")));
    let mut wired = unbound;
    wired.panel.links.push(crab_core::WireLink {
        from: "K1:A2".to_owned(),
        to: "bus:0V".to_owned(),
    });
    assert!(!errors(&wired)
        .iter()
        .any(|m| m.contains("не привязано к входу или выходу проекта")));
}

#[test]
fn a_bus_end_names_a_line_of_the_panel() {
    // On a 230 V panel the lines are the 24 V rails and `L`, `N`.
    let with = |link: &str| {
        pump_with(&format!(
            "supply = \"ac230\"\n{GOOD}\n[[link]]\nfrom = \"K1:A2\"\nto = \"{link}\"\n"
        ))
    };
    for good in ["bus:0V", "bus:+24V", "bus:L", "bus:N"] {
        let found = errors(&with(good));
        assert!(
            found.iter().all(|m| !m.contains("такой шины нет")),
            "{good}: {found:#?}"
        );
    }
    let found = errors(&with("bus:X9"));
    let wanted = "провод -K1:A2 — bus:X9: такой шины нет (есть: +24V, 0V, L, N)";
    assert_eq!(
        found.iter().filter(|m| m.as_str() == wanted).count(),
        1,
        "{found:#?}"
    );
}
