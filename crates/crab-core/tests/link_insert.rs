//! Putting a block into a link (`specs/link-insert`, task 1), on the pump example.

use std::path::PathBuf;

use crab_core::{BlockPos, EditError, Insert, Library, PortRef, Project};

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn setup() -> (Library, Project) {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let project = Project::load(repo_path("examples/pump")).unwrap();
    (lib, project)
}

fn pos() -> BlockPos {
    BlockPos {
        sheet: 1,
        x: 100.0,
        y: 100.0,
    }
}

fn port(s: &str) -> PortRef {
    s.parse().unwrap()
}

fn has_link(p: &Project, from: &str, to: &str) -> bool {
    p.logic
        .links
        .iter()
        .any(|l| l.from == port(from) && l.to == port(to))
}

/// The bool link `low.out → delay.in`.
fn bool_link() -> PortRef {
    port("delay.in")
}

#[test]
fn a_block_with_input_and_output_cuts_the_link() {
    let (lib, mut p) = setup();
    let id = p.add_block(&lib, "not", pos()).unwrap();
    let r = p.insert_into_link(&lib, &id, &bool_link()).unwrap();
    assert_eq!(
        r,
        Insert::Spliced {
            input: "in".into(),
            output: "out".into()
        }
    );
    assert!(!has_link(&p, "low.out", "delay.in"));
    assert!(has_link(&p, "low.out", &format!("{id}.in")));
    assert!(has_link(&p, &format!("{id}.out"), "delay.in"));
}

#[test]
fn a_block_with_several_inputs_takes_the_first_free_one_of_the_right_type() {
    let (lib, mut p) = setup();
    let id = p.add_block(&lib, "and", pos()).unwrap();
    p.insert_into_link(&lib, &id, &bool_link()).unwrap();
    assert!(has_link(&p, "low.out", &format!("{id}.a")));
    assert!(!p
        .logic
        .links
        .iter()
        .any(|l| l.to == port(&format!("{id}.b"))));
}

#[test]
fn a_taken_input_is_skipped() {
    let (lib, mut p) = setup();
    let id = p.add_block(&lib, "and", pos()).unwrap();
    p.set_input_const(
        &lib,
        &port(&format!("{id}.a")),
        Some(crab_core::ParamValue::Bool(true)),
    )
    .unwrap();
    p.insert_into_link(&lib, &id, &bool_link()).unwrap();
    assert!(has_link(&p, "low.out", &format!("{id}.b")));
}

#[test]
fn a_block_without_input_replaces_the_link() {
    let (lib, mut p) = setup();
    let id = p.add_block(&lib, "digital_in", pos()).unwrap();
    let r = p.insert_into_link(&lib, &id, &bool_link()).unwrap();
    assert_eq!(
        r,
        Insert::Replaced {
            output: "out".into()
        }
    );
    assert!(!has_link(&p, "low.out", "delay.in"));
    assert!(has_link(&p, &format!("{id}.out"), "delay.in"));
}

#[test]
fn a_block_without_output_branches_the_link() {
    let (lib, mut p) = setup();
    let id = p.add_block(&lib, "digital_out", pos()).unwrap();
    let r = p.insert_into_link(&lib, &id, &bool_link()).unwrap();
    assert_eq!(r, Insert::Branch { input: "in".into() });
    assert!(has_link(&p, "low.out", "delay.in"));
    assert!(has_link(&p, "low.out", &format!("{id}.in")));
}

#[test]
fn nothing_fits_by_type_means_a_refusal_without_changes() {
    let (lib, mut p) = setup();
    // A bool block on the float link `pressure.out → low.a`.
    let id = p.add_block(&lib, "not", pos()).unwrap();
    let before = p.clone();
    let err = p.insert_into_link(&lib, &id, &port("low.a")).unwrap_err();
    assert_eq!(err, EditError::InsertNoPort(id));
    assert_eq!(p, before);
}

#[test]
fn a_missing_link_and_the_own_link_are_refused() {
    let (lib, mut p) = setup();
    let id = p.add_block(&lib, "not", pos()).unwrap();
    assert_eq!(
        p.insert_into_link(&lib, &id, &port("pump.en")),
        Err(EditError::NoLink(port("pump.en")))
    );
    // `delay` is the end of the link: it cannot be put into it.
    assert_eq!(
        p.insert_into_link(&lib, "delay", &bool_link()),
        Err(EditError::InsertOwnLink("delay".into()))
    );
}

#[test]
fn the_inverted_input_of_the_target_is_kept() {
    let (lib, mut p) = setup();
    p.set_port_inverted(&lib, &port("delay.in"), false, true)
        .unwrap();
    let id = p.add_block(&lib, "not", pos()).unwrap();
    p.insert_into_link(&lib, &id, &bool_link()).unwrap();
    let delay = p.logic.block("delay").unwrap();
    assert!(delay.inverted_inputs.contains("in"));
}
