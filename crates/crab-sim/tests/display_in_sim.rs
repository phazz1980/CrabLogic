//! The display driver runs inside the schema simulation: the screen of the `oled` block and the
//! menu (opened by the `ok` button of the menu block) are drawn on the chip model.

use std::path::{Path, PathBuf};

use crab_codegen::Program;
use crab_core::{Library, Project};
use crab_sim::{DisplayDrive, DisplayView, Simulation};

const LOGIC: &str = include_str!("fixtures/display_menu.toml");

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

fn simulation(name: &str, ok: bool) -> Simulation {
    let root = std::env::temp_dir().join(format!("crab_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let lib_dir = root.join("lib");
    for block in [
        "oled",
        "menu",
        "menu_item",
        "disp_text",
        "disp_value",
        "disp_bar",
    ] {
        copy_dir(&repo_path("library").join(block), &lib_dir.join(block));
    }
    let (lib, errors) = Library::load(&lib_dir).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let dir = root.join("project");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"sim\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    )
    .unwrap();
    let logic = LOGIC
        .replace("up = UP_VALUE", "up = false")
        .replace("down = DOWN_VALUE", &format!("down = false\nok = {ok}"));
    std::fs::write(dir.join("logic.toml"), logic).unwrap();
    let project = Project::load(&dir).unwrap();
    let mut sim = Simulation::new(&project.logic, &lib).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    for id in program.display_configs().keys() {
        let plan = program.display_sim_plan(id).unwrap();
        sim.drive_display(
            id,
            &DisplayDrive {
                config: plan.config,
                widgets: plan.widgets,
                string_widgets: plan.string_widgets,
                menu: plan.menu,
            },
        )
        .unwrap();
    }
    let _ = std::fs::remove_dir_all(&root);
    sim
}

fn lit_count(v: &DisplayView) -> usize {
    v.pixels.iter().filter(|p| **p).count()
}

#[test]
fn screen_is_drawn_in_the_simulation() {
    let mut sim = simulation("sim_screen", false);
    for _ in 0..40 {
        sim.step(10).unwrap();
    }
    let v = sim.displays().remove(0);
    assert_eq!((v.width, v.height), (128, 64));
    assert!(
        lit_count(&v) > 20,
        "the screen has widgets: {}",
        lit_count(&v)
    );
}

#[test]
fn menu_opens_with_the_ok_button() {
    let mut closed = simulation("sim_closed", false);
    let mut open = simulation("sim_open", true);
    for _ in 0..40 {
        closed.step(10).unwrap();
        open.step(10).unwrap();
    }
    let (a, b) = (closed.displays().remove(0), open.displays().remove(0));
    assert_ne!(a.pixels, b.pixels, "the menu replaces the screen");
    assert!(
        (0..128).all(|x| b.pixels[8 * 128 + x]),
        "the menu title line is drawn"
    );
}
