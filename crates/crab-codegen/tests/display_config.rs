//! The `crab.config` blob of display blocks (`specs/display`): screens and widgets of the
//! constructor, the menu tree, the slots the generator and the driver agree on.

use std::path::PathBuf;

use crab_codegen::Program;
use crab_core::{Library, Project};
use crab_ui_format::{
    Config, ConfigView, DisplayKind, Menu, MenuItem, MenuItemKind, Screen, Widget, WidgetKind,
};

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// One OLED with two screens (text, value and bar widgets) and a two-level menu with a number
/// item on a retained float variable.
pub const PROJECT: &str = r#"
[[var]]
name = "setpoint"
type = "float"
init = 20.0
retain = true

[[var]]
name = "enabled"
type = "bool"

[[var]]
name = "level"
type = "float"
init = 3.0

[[block]]
id = "oled"
type = "oled"

[[block]]
id = "title"
type = "disp_text"
display = "oled"
screen = 1
text = "Насос"
x = 0
y = 0
w = 64
h = 8

[[block]]
id = "temp"
type = "disp_value"
display = "oled"
screen = 1
x = 0
y = 16
w = 48
h = 8
decimals = 2
unit = " C"
[block.const]
in = 5.0

[[block]]
id = "bar"
type = "disp_bar"
display = "oled"
screen = 2
x = 0
y = 8
w = 64
h = 8
min = 0.0
max = 10.0
[block.bind]
in = "level"

[[block]]
id = "m"
type = "menu"
display = "oled"
timeout = 20

[[block]]
id = "en"
type = "menu_item"
menu = "m"
kind = "bool"
caption = "Работа"
var = "enabled"
order = 2

[[block]]
id = "sp"
type = "menu_item"
menu = "m"
parent = "sub"
kind = "number"
caption = "Уставка"
var = "setpoint"
min = 0.0
max = 100.0
step = 0.5
unit = " C"
order = 1

[[block]]
id = "sub"
type = "menu_item"
menu = "m"
kind = "submenu"
caption = "Настройки"
order = 1
"#;

fn project() -> (Library, Project) {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let dir = std::env::temp_dir().join(format!("crab_display_config_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.toml"),
        "[project]\nname = \"display_config\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("logic.toml"), PROJECT).unwrap();
    let project = Project::load(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    (lib, project)
}

fn widget(kind: WidgetKind, x: u8, y: u8, w: u8, input: Option<u16>) -> Widget {
    Widget {
        kind,
        x,
        y,
        w,
        h: 8,
        font: 8,
        input,
    }
}

#[test]
fn config_of_an_oled_with_two_screens_and_a_menu() {
    let (lib, project) = project();
    let program = Program::build(&project, &lib).unwrap();
    let config = program.display_config("oled").unwrap();
    let mut expected = Config::new(DisplayKind::Ssd1306_128x64);
    expected.screens = vec![
        Screen {
            widgets: vec![
                widget(
                    WidgetKind::Text {
                        text: "Насос".into(),
                    },
                    0,
                    0,
                    64,
                    None,
                ),
                widget(
                    WidgetKind::Value {
                        format: 0,
                        decimals: 2,
                        unit: " C".into(),
                    },
                    0,
                    16,
                    48,
                    Some(0),
                ),
            ],
        },
        Screen {
            widgets: vec![widget(
                WidgetKind::Bar {
                    min: 0.0,
                    max: 10.0,
                },
                0,
                8,
                64,
                Some(1),
            )],
        },
    ];
    expected.menu = Some(Menu {
        timeout_s: 20,
        items: vec![
            MenuItem {
                parent: None,
                kind: MenuItemKind::Submenu,
                caption: "Настройки".into(),
                var_slot: None,
            },
            MenuItem {
                parent: Some(0),
                kind: MenuItemKind::Number {
                    min: 0.0,
                    max: 100.0,
                    step: 0.5,
                    decimals: 1,
                    unit: " C".into(),
                },
                caption: "Уставка".into(),
                var_slot: Some(0),
            },
            MenuItem {
                parent: None,
                kind: MenuItemKind::Bool,
                caption: "Работа".into(),
                var_slot: Some(1),
            },
        ],
    });
    assert_eq!(config, expected);

    // The bytes the driver gets are the encoding of this structure and parse back.
    let bytes = program.display_configs();
    assert_eq!(bytes.keys().collect::<Vec<_>>(), ["oled"]);
    assert_eq!(bytes["oled"], expected.encode());
    assert!(bytes["oled"].len() < crab_codegen::MAX_BLOCK_CONFIG);
    ConfigView::parse(&bytes["oled"]).unwrap();
}

#[test]
fn project_without_displays_has_no_configs() {
    let (lib, errors) = Library::load(repo_path("library")).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let project = Project::load(repo_path("examples/background")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    assert!(program.display_configs().is_empty());
    assert!(program.display_config("anything").is_none());
}
