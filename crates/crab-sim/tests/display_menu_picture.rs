//! The real OLED driver (`library/oled/block.wasm`) runs in the simulator and draws the graphic
//! menu on the SSD1306 model: title line, marker at the selected item, hint bar.

use std::path::{Path, PathBuf};

use crab_codegen::Program;
use crab_core::{Library, Project};
use crab_sim::mcu::{Manifest, McuModule, StubIo};
use crab_sim::{DisplayDevice, DisplayView};

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

/// Runs the sheet of the menu fixture (`ok` held from the first step, so the menu opens) with
/// the real driver and returns the display after enough steps for a whole frame.
fn picture(name: &str, menu_params: &str) -> DisplayView {
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
        "[project]\nname = \"menu_picture\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    )
    .unwrap();
    let logic = LOGIC
        .replace("up = UP_VALUE", "up = false")
        .replace("down = DOWN_VALUE", "down = false\nok = true")
        .replace(
            "[block.const]\nup",
            &format!("{menu_params}[block.const]\nup"),
        );
    std::fs::write(dir.join("logic.toml"), logic).unwrap();
    let project = Project::load(&dir).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut io = StubIo::default();
    io.displays
        .push(DisplayDevice::for_block("oled", "ssd1306_128x64", "oled", 60).unwrap());
    let mut m = McuModule::load_with(&built.wasm, &manifest, built.watched.len(), io).unwrap();
    for _ in 0..40 {
        m.step(10).unwrap();
    }
    let view = m.displays().remove(0);
    let _ = std::fs::remove_dir_all(&root);
    view
}

fn on(v: &DisplayView, x: usize, y: usize) -> bool {
    v.pixels[y * v.width + x]
}

fn lit(v: &DisplayView, x: usize, y: usize, w: usize, h: usize) -> bool {
    (y..y + h).all(|yy| (x..x + w).all(|xx| on(v, xx, yy)))
}

fn dark(v: &DisplayView, x: usize, y: usize, w: usize, h: usize) -> bool {
    (y..y + h).all(|yy| (x..x + w).all(|xx| !on(v, xx, yy)))
}

#[test]
fn graphic_menu_on_the_oled_model() {
    // The fixture menu has two root items, the submenu «Настройки» and the switch «Работа»; defaults: 16 px, hints on.
    let v = picture("menu_picture_16", "");
    assert!((v.width, v.height) == (128, 64));
    assert!(lit(&v, 0, 8, 128, 1), "line under the title");
    assert!(lit(&v, 0, 14, 5, 5), "5x5 marker of the selected item");
    assert!(!dark(&v, 8, 10, 12, 14), "caption");
    assert!(!dark(&v, 118, 10, 8, 14), "submenu mark at the right edge");
    assert!(!lit(&v, 127, 11, 1, 40), "no scrollbar: two items fit");
    for i in 0..4 {
        assert!(lit(&v, 1 + i * 32, 56, 3, 8), "hint cell {i}");
        assert!(dark(&v, i * 32, 57, 1, 7), "gap {i}");
    }
}

#[test]
fn menu_params_font_and_hints_reach_the_driver() {
    let v = picture("menu_picture_8", "font = 8\nhints = false\n");
    assert!(lit(&v, 0, 12, 4, 4), "4x4 marker in the 8 px font");
    assert!(dark(&v, 0, 56, 128, 8), "no hint bar");
}
