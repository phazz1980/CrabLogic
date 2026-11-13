//! The real display drivers (`library/oled`, `library/lcd`) draw a `disp_string` widget: the text
//! of a string signal, clipped to the widget width; an unconnected widget shows nothing.

use std::path::{Path, PathBuf};

use crab_codegen::Program;
use crab_core::{Library, Project};
use crab_sim::mcu::{Manifest, McuModule, StubIo};
use crab_sim::{DisplayDevice, DisplayView};

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

/// Runs a sheet with `display` (`oled` or `lcd`), a constant string `Привет` feeding a string
/// widget at x = 1, y = 0 of width `w`, and a second, unconnected string widget below it.
fn picture(name: &str, display: &str, w: u32) -> DisplayView {
    let root = std::env::temp_dir().join(format!("crab_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let lib_dir = root.join("lib");
    for block in [display, "disp_string", "text_const"] {
        copy_dir(&repo_path("library").join(block), &lib_dir.join(block));
    }
    let (lib, errors) = Library::load(&lib_dir).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let dir = root.join("project");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"display_string\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    )
    .unwrap();
    let (model, address, y2, h) = if display == "oled" {
        ("ssd1306_128x64", 60, 16, 8)
    } else {
        ("1602", 39, 1, 1)
    };
    let logic = format!(
        r#"
[[block]]
id = "scr"
type = "{display}"
model = "{model}"
address = {address}

[[block]]
id = "txt"
type = "text_const"
value = "Привет"

[[block]]
id = "s1"
type = "disp_string"
display = "scr"
x = 1
y = 0
w = {w}
h = {h}

[[block]]
id = "s2"
type = "disp_string"
display = "scr"
x = 1
y = {y2}
w = {w}
h = {h}

[[link]]
from = "txt.out"
to = "s1.in"
"#
    );
    std::fs::write(dir.join("logic.toml"), logic).unwrap();
    let project = Project::load(&dir).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module_with(1, true).unwrap();
    let manifest = Manifest::parse(&built.manifest).unwrap();
    let mut io = StubIo::default();
    io.displays
        .push(DisplayDevice::for_block(display, model, "scr", address).unwrap());
    let mut m = McuModule::load_with(&built.wasm, &manifest, built.watched.len(), io).unwrap();
    for _ in 0..60 {
        m.step(60).unwrap();
    }
    let view = m.displays().remove(0);
    let _ = std::fs::remove_dir_all(&root);
    view
}

fn count(v: &DisplayView, x: usize, y: usize, w: usize, h: usize) -> usize {
    (y..y + h)
        .flat_map(|yy| (x..x + w).map(move |xx| (xx, yy)))
        .filter(|&(xx, yy)| v.pixels[yy * v.width + xx])
        .count()
}

#[test]
fn oled_draws_the_string_clipped_to_the_widget() {
    // 24 px hold four 6 px characters: «Прив» at x = 1..25, nothing after, nothing below.
    let v = picture("string_oled", "oled", 24);
    assert!(count(&v, 1, 0, 24, 8) > 20, "text is drawn");
    assert_eq!(count(&v, 25, 0, 100, 8), 0, "clipped to the width");
    assert_eq!(
        count(&v, 0, 8, 128, 56),
        0,
        "the unconnected widget is empty"
    );
    // Wide enough: the whole word is there.
    let wide = picture("string_oled_wide", "oled", 60);
    assert!(count(&wide, 25, 0, 12, 8) > 0, "«ве» of «Привет»");
    assert_eq!(count(&wide, 38, 0, 90, 8), 0);
}

#[test]
fn lcd_draws_the_string_clipped_to_the_widget() {
    let v = picture("string_lcd", "lcd", 4);
    assert_eq!(v.rows[0].len(), 16);
    let codes = &v.codes[0];
    // Cells 1..5 hold «Прив» (custom glyphs for П, и, в; р looks like Latin p), the rest is blank.
    assert!(codes[1..5].iter().all(|&c| c != b' '), "{codes:?}");
    assert_eq!(codes[0], b' ');
    assert!(codes[5..].iter().all(|&c| c == b' '), "{codes:?}");
    assert!(v.rows[1].trim().is_empty());
}
