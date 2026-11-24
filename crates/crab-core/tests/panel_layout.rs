//! Manual layout of the panel sheets stored with the project (`panel-layout.toml`).

use std::fs::File;
use std::path::{Path, PathBuf};

use crab_core::panel::{ContactForm, Device, DeviceKind};
use crab_core::{
    Layout, Logic, Panel, PanelLayout, Project, ProjectConfig, SheetId, PANEL_LAYOUT_FILE,
};
use zip::ZipArchive;

fn config() -> ProjectConfig {
    toml::from_str("[project]\nname = \"t\"\n[board]\nfqbn = \"x:y:z\"\n").unwrap()
}

fn panel() -> Panel {
    let device = |id: &str, kind| Device {
        id: id.into(),
        kind,
        rating: String::new(),
        part: String::new(),
        bind: String::new(),
        load: String::new(),
        poles: None,
        of: String::new(),
        form: ContactForm::default(),
        pins: String::new(),
        from: String::new(),
        free: false,
        series: String::new(),
        conductors: Vec::new(),
    };
    Panel {
        devices: vec![
            device("K1", DeviceKind::Contactor),
            device("M1", DeviceKind::Motor),
        ],
        ..Default::default()
    }
}

fn project_in(dir: &Path, name: &str) -> Project {
    Project {
        path: dir.join(format!("{name}.crab")),
        config: config(),
        logic: Logic::default(),
        layout: Layout::default(),
        panel: panel(),
        panel_layout: PanelLayout::default(),
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("crab-panel-layout-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn archive_has(path: &Path, entry: &str) -> bool {
    let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
    let found = zip.by_name(entry).is_ok();
    found
}

#[test]
fn rename_device_moves_the_layout_with_the_panel() {
    let dir = scratch("rename-device");
    let mut project = project_in(&dir, "rename");
    project
        .panel_layout
        .place("K1", SheetId::Power(0), 20.0, 20.0);
    project.panel_layout.set_shift("power:K1", 5.0, 5.0);
    project.panel_layout.set_bend("K1:2>M1:L1", Some(10.0));
    crab_core::rename_device(&mut project, "K1", "K5").unwrap();
    assert!(project.panel.device("K5").is_some() && project.panel.device("K1").is_none());
    assert_eq!(
        project.panel_layout.placed.keys().collect::<Vec<_>>(),
        vec!["K5"]
    );
    assert_eq!(project.panel_layout.shift("power:K5"), (5.0, 5.0));
    assert_eq!(project.panel_layout.shift("power:K1"), (0.0, 0.0));
    assert_eq!(project.panel_layout.bend("K5:2>M1:L1"), Some(10.0));
    assert_eq!(project.panel_layout.bend("K1:2>M1:L1"), None);
}

#[test]
fn refused_rename_leaves_the_panel_and_the_layout_unchanged() {
    let dir = scratch("rename-refused");
    let mut project = project_in(&dir, "refused");
    project.panel_layout.set_shift("power:K1", 5.0, 5.0);
    project.panel_layout.set_bend("K1:2>M1:L1", Some(10.0));
    let panel_before = project.panel.clone();
    let layout_before = project.panel_layout.clone();
    // M1 is taken by the motor.
    assert!(crab_core::rename_device(&mut project, "K1", "M1").is_err());
    assert_eq!(project.panel, panel_before);
    assert_eq!(project.panel_layout, layout_before);
}

#[test]
fn rename_device_moves_the_placed_coil_part() {
    let dir = scratch("rename-coil");
    let mut project = project_in(&dir, "coil");
    project
        .panel_layout
        .place("K1/coil", SheetId::Control(0), 40.0, 20.0);
    crab_core::rename_device(&mut project, "K1", "K5").unwrap();
    assert_eq!(
        project.panel_layout.placed.keys().collect::<Vec<_>>(),
        vec!["K5/coil"]
    );
    assert_eq!(project.panel_layout.placed["K5/coil"].x, 40.0);
}

#[test]
fn shifts_and_bends_snap_and_disappear_at_zero() {
    let mut layout = PanelLayout::default();
    let key = crab_core::symbol_key("main", "K1");
    assert_eq!(key, "main:K1");
    layout.set_shift(&key, 6.0, -4.0);
    assert_eq!(layout.shift(&key), (5.0, -5.0));
    layout.set_shift(&key, 0.0, 0.0);
    assert!(layout.is_empty());

    layout.set_bend("K1:2>M1:L1", Some(12.0));
    assert_eq!(layout.bend("K1:2>M1:L1"), Some(10.0));
    layout.set_bend("K1:2>M1:L1", None);
    assert!(layout.is_empty());
}

#[test]
fn project_saves_and_loads_the_layout_in_the_archive() {
    let dir = scratch("archive");
    let mut project = project_in(&dir, "t");
    project
        .panel_layout
        .set_shift(&crab_core::symbol_key("main", "K1"), 10.0, 5.0);
    project.panel_layout.set_bend("K1:2>M1:L1", Some(-15.0));
    project.save().unwrap();

    assert!(archive_has(&project.path, PANEL_LAYOUT_FILE));
    let again = Project::load(&project.path).unwrap();
    assert_eq!(again.panel_layout, project.panel_layout);
    assert_eq!(again, project);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_keeps_sheet_names_captions_and_placed_devices_in_the_archive() {
    let dir = scratch("sheets");
    let mut project = project_in(&dir, "t");
    project
        .panel_layout
        .set_caption(SheetId::Free(1), "  Pump drive  ");
    project
        .panel_layout
        .place("K1", SheetId::Free(1), 40.0, 20.0);
    project.save().unwrap();

    let again = Project::load(&project.path).unwrap();
    assert_eq!(
        again.panel_layout.caption(SheetId::Free(1)),
        Some("Pump drive")
    );
    assert_eq!(again.panel_layout.caption(SheetId::Free(0)), None);
    assert_eq!(again.panel_layout.placed["K1"].sheet, SheetId::Free(1));
    assert_eq!(again, project);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn folder_layout_of_the_old_numbering_reads_as_power_sheet_names() {
    let dir = scratch("legacy-version");
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"t\"\n[board]\nfqbn = \"x:y:z\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("logic.toml"), "").unwrap();
    std::fs::write(
        dir.join("panel.toml"),
        "[[device]]\nid = \"K1\"\nkind = \"contactor\"\n[[device]]\nid = \"M1\"\nkind = \"motor\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join(PANEL_LAYOUT_FILE),
        "[placed.\"K1\"]\nsheet = 0\nx = 10.0\ny = 10.0\npower = true\n",
    )
    .unwrap();

    let project = Project::load(&dir).unwrap();
    assert_eq!(project.panel_layout.placed["K1"].sheet, SheetId::Power(0));
    // Saving writes the name, not the old flag.
    project.save().unwrap();
    let text = {
        let mut zip = ZipArchive::new(File::open(&project.path).unwrap()).unwrap();
        let mut entry = zip.by_name(PANEL_LAYOUT_FILE).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut entry, &mut text).unwrap();
        text
    };
    assert!(text.contains("sheet = \"power:0\""), "{text}");
    assert!(!text.contains("power = "), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_loads_the_layout_from_a_folder() {
    let dir = scratch("folder");
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"t\"\n[board]\nfqbn = \"x:y:z\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("logic.toml"), "").unwrap();
    std::fs::write(
        dir.join("panel.toml"),
        "[[device]]\nid = \"K1\"\nkind = \"contactor\"\n[[device]]\nid = \"M1\"\nkind = \"motor\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join(PANEL_LAYOUT_FILE),
        "[symbols.\"main:K1\"]\ndx = 10.0\ndy = -5.0\n[wires.\"K1:2>M1:L1\"]\nbend = 20.0\n",
    )
    .unwrap();

    let project = Project::load(&dir).unwrap();
    assert_eq!(project.panel_layout.shift("main:K1"), (10.0, -5.0));
    assert_eq!(project.panel_layout.bend("K1:2>M1:L1"), Some(20.0));

    // Saving converts the folder to the archive and keeps the layout.
    project.save().unwrap();
    assert!(archive_has(&project.path, PANEL_LAYOUT_FILE));
    assert_eq!(
        Project::load(&project.path).unwrap().panel_layout,
        project.panel_layout
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn old_project_without_the_file_loads_with_an_empty_layout() {
    let dir = scratch("old");
    let project = project_in(&dir, "t");
    project.save().unwrap();
    assert!(!archive_has(&project.path, PANEL_LAYOUT_FILE));
    let again = Project::load(&project.path).unwrap();
    assert!(again.panel_layout.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn empty_layout_writes_no_file_and_a_layout_of_removed_devices_is_pruned() {
    let dir = scratch("prune");
    let mut project = project_in(&dir, "t");
    // Only entries of a device that the panel no longer has: pruned, so nothing is written.
    project
        .panel_layout
        .set_shift(&crab_core::symbol_key("main", "G9"), 5.0, 5.0);
    project.save().unwrap();
    assert!(!archive_has(&project.path, PANEL_LAYOUT_FILE));
    // The project keeps the entry in memory; only the file leaves it out.
    assert!(!project.panel_layout.is_empty());

    project
        .panel_layout
        .set_shift(&crab_core::symbol_key("main", "K1"), 5.0, 5.0);
    project.save().unwrap();
    let again = Project::load(&project.path).unwrap();
    assert_eq!(again.panel_layout.shift("main:K1"), (5.0, 5.0));
    assert_eq!(again.panel_layout.shift("main:G9"), (0.0, 0.0));
    let _ = std::fs::remove_dir_all(&dir);
}
