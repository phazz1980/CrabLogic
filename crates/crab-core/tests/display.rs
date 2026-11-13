//! Displays, screen widgets and menus: library blocks, tags of widgets and checks.
use std::path::PathBuf;

use crab_core::{check, DiagKind, Library, Logic, Project, Report, Severity};

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

fn run(src: &str) -> Report {
    let logic: Logic = toml::from_str(src).unwrap();
    check(&logic, &library(), None)
}

fn kinds(report: &Report) -> Vec<DiagKind> {
    report.diagnostics.iter().map(|d| d.kind).collect()
}

fn has(report: &Report, kind: DiagKind) -> bool {
    kinds(report).contains(&kind)
}

const OLED: &str = r#"
[[block]]
id = "scr"
type = "oled"
"#;

#[test]
fn display_blocks_are_in_the_library() {
    let lib = library();
    for kind in [
        "oled",
        "lcd",
        "menu",
        "menu_item",
        "disp_text",
        "disp_value",
        "disp_bar",
        "disp_led",
        "disp_icon",
    ] {
        assert!(lib.get(kind).is_some(), "{kind}");
    }
    // Without firmware yet: no template, no module.
    assert_eq!(
        lib.get("disp_value").unwrap().firmware_source(),
        crab_core::library::FirmwareSource::None
    );
}

#[test]
fn widget_inside_screen_is_valid() {
    let src = format!(
        "{OLED}\n[[block]]\nid = \"t\"\ntype = \"disp_text\"\ndisplay = \"scr\"\nx = 0\ny = 56\nw = 128\nh = 8\n"
    );
    let report = run(&src);
    assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
}

#[test]
fn widget_outside_screen() {
    // 128x32 OLED: the second text line at y = 32 does not fit.
    let src = r#"
[[block]]
id = "scr"
type = "oled"
model = "ssd1306_128x32"

[[block]]
id = "t"
type = "disp_text"
display = "scr"
y = 32
"#;
    let report = run(src);
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.kind == DiagKind::DisplayWidget)
        .expect("widget outside the screen");
    assert_eq!(d.severity, Severity::Error);
    assert!(d.message.contains("128×32"), "{}", d.message);

    // LCD 1602: width counts in character cells.
    let src = r#"
[[block]]
id = "lcd"
type = "lcd"

[[block]]
id = "t"
type = "disp_text"
display = "lcd"
x = 10
w = 10
h = 1
"#;
    assert!(has(&run(src), DiagKind::DisplayWidget));
    let ok = src.replace("x = 10", "x = 6");
    assert!(!has(&run(&ok), DiagKind::DisplayWidget));
}

#[test]
fn widget_without_display() {
    let missing = "[[block]]\nid = \"t\"\ntype = \"disp_text\"\ndisplay = \"nope\"\n";
    assert!(has(&run(missing), DiagKind::DisplayWidget));
    let not_display = format!(
        "{}\n[[block]]\nid = \"n\"\ntype = \"disp_text\"\ndisplay = \"d\"\n",
        "[[block]]\nid = \"d\"\ntype = \"menu\"\ndisplay = \"\""
    );
    assert!(has(&run(&not_display), DiagKind::DisplayWidget));
}

#[test]
fn two_displays_at_one_address() {
    let src = r#"
[[block]]
id = "a"
type = "oled"

[[block]]
id = "b"
type = "oled"
"#;
    let report = run(src);
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.kind == DiagKind::DisplayAddress)
        .expect("address clash");
    assert_eq!(d.block.as_deref(), Some("b"));
    assert!(d.message.contains("0x3C"), "{}", d.message);

    // Another address, or another bus: fine.
    let other_address = src.replace(
        "id = \"b\"\ntype = \"oled\"",
        "id = \"b\"\ntype = \"oled\"\naddress = 61",
    );
    assert!(!has(&run(&other_address), DiagKind::DisplayAddress));
    let other_bus = src.replace(
        "id = \"b\"\ntype = \"oled\"",
        "id = \"b\"\ntype = \"oled\"\nsda = 4\nscl = 5",
    );
    assert!(!has(&run(&other_bus), DiagKind::DisplayAddress));
}

#[test]
fn menu_needs_a_display() {
    let none = "[[block]]\nid = \"m\"\ntype = \"menu\"\n";
    assert!(has(&run(none), DiagKind::MenuDisplay));
    let missing = "[[block]]\nid = \"m\"\ntype = \"menu\"\ndisplay = \"nope\"\n";
    assert!(has(&run(missing), DiagKind::MenuDisplay));
    let wrong = format!(
        "[[block]]\nid = \"m\"\ntype = \"menu\"\ndisplay = \"w\"\n\n[[block]]\nid = \"w\"\ntype = \"disp_led\"\ndisplay = \"scr\"\n{OLED}"
    );
    assert!(has(&run(&wrong), DiagKind::MenuDisplay));
    let ok = format!("{OLED}\n[[block]]\nid = \"m\"\ntype = \"menu\"\ndisplay = \"scr\"\n");
    assert!(!has(&run(&ok), DiagKind::MenuDisplay));
}

#[test]
fn lcd_cyrillic_limit_is_a_warning() {
    let src = |text: &str| {
        format!(
            "[[block]]\nid = \"lcd\"\ntype = \"lcd\"\nmodel = \"2004\"\n\n[[block]]\nid = \"t\"\ntype = \"disp_text\"\ndisplay = \"lcd\"\ntext = \"{text}\"\nw = 20\nh = 1\n"
        )
    };
    // Eight different letters (repeats do not count): no warning.
    assert!(!has(&run(&src("Давление ааа")), DiagKind::DisplayCyrillic));
    // More than eight different letters.
    let report = run(&src("Температура воздуха"));
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.kind == DiagKind::DisplayCyrillic)
        .expect("too many letters");
    assert_eq!(d.severity, Severity::Warning);
    assert!(!report.has_errors(), "{:#?}", report.diagnostics);
}

#[test]
fn oled_has_no_cyrillic_limit() {
    let src = format!(
        "{OLED}\n[[block]]\nid = \"t\"\ntype = \"disp_text\"\ndisplay = \"scr\"\ntext = \"Температура воздуха\"\nw = 128\n"
    );
    assert!(!has(&run(&src), DiagKind::DisplayCyrillic));
}

#[test]
fn menu_item_variable_types() {
    let vars = r#"
[[var]]
name = "setpoint"
type = "float"

[[var]]
name = "mode"
type = "int"

[[var]]
name = "enabled"
type = "bool"
"#;
    let item = |kind: &str, var: &str| {
        format!("{vars}\n[[block]]\nid = \"i\"\ntype = \"menu_item\"\nkind = \"{kind}\"\nvar = \"{var}\"\n")
    };
    for (kind, var) in [
        ("number", "setpoint"),
        ("number", "mode"),
        ("bool", "enabled"),
        ("choice", "mode"),
        ("action", "enabled"),
        ("submenu", ""),
        ("view", "setpoint"),
    ] {
        let report = run(&item(kind, var));
        assert!(!has(&report, DiagKind::MenuItemVar), "{kind} {var}");
    }
    for (kind, var) in [
        ("number", "enabled"),
        ("bool", "mode"),
        ("choice", "setpoint"),
        ("number", "nope"),
        ("number", ""),
    ] {
        let report = run(&item(kind, var));
        assert!(has(&report, DiagKind::MenuItemVar), "{kind} {var}");
    }
}

fn widget_project() -> Project {
    let mut p = Project::load(repo_path("examples/pump")).unwrap();
    let dir = std::env::temp_dir().join(format!("crab-disp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    p.path = dir.join("screen.crab");
    for (id, kind) in [
        ("scr", "oled"),
        ("temp", "disp_value"),
        ("item", "menu_item"),
    ] {
        p.logic.blocks.push(crab_core::Block {
            id: id.into(),
            kind: kind.into(),
            ..Default::default()
        });
    }
    p.logic
        .blocks
        .iter_mut()
        .find(|b| b.id == "temp")
        .unwrap()
        .params
        .insert("display".into(), crab_core::ParamValue::Text("scr".into()));
    p
}

#[test]
fn screen_widgets_are_tags_stored_in_web_toml() {
    let lib = library();
    let p = widget_project();
    assert!(p.is_widget("temp") && p.is_widget("item"));
    assert!(!p.is_widget("scr"));
    // The widget input is not reported as unfed: its images on the sheets are.
    let report = crab_core::check::check_project(&p, &lib);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.kind == DiagKind::InputEmpty && d.block.as_deref() == Some("temp")),
        "{:#?}",
        report.diagnostics
    );

    p.save().unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&p.path).unwrap()).unwrap();
    let mut read = |name: &str| {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut zip.by_name(name).unwrap(), &mut text).unwrap();
        text
    };
    let web = read("web.toml");
    assert!(web.contains("disp_value") && web.contains("menu_item"));
    assert!(!web.contains("oled"));
    let logic = read("logic.toml");
    assert!(!logic.contains("disp_value") && logic.contains("oled"));
    let back = Project::load(&p.path).unwrap();
    assert_eq!(back.logic.blocks.len(), p.logic.blocks.len());
    std::fs::remove_dir_all(p.path.parent().unwrap()).unwrap();
}

#[test]
fn screen_widget_rename_and_delete_follow_web_widgets() {
    let mut p = widget_project();
    p.logic.links.push(crab_core::Link {
        from: crab_core::PortRef::new("pressure", "out"),
        to: crab_core::PortRef::new("temp", "in"),
    });
    // Renaming the display moves the widgets that name it.
    p.rename_block("scr", "oled1").unwrap();
    assert_eq!(
        p.logic.block("temp").unwrap().params.get("display"),
        Some(&crab_core::ParamValue::Text("oled1".into()))
    );
    // Renaming the widget keeps its link.
    p.rename_block("temp", "t_air").unwrap();
    assert!(p.logic.links.iter().any(|l| l.to.block == "t_air"));
    // Deleting the widget leaves its link for the check to report.
    p.remove_block("t_air").unwrap();
    assert!(p.logic.block("t_air").is_none());
    assert!(p.logic.links.iter().any(|l| l.to.block == "t_air"));
}
