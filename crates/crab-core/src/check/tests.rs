#[test]
fn diag_kind_ids_are_unique_kebab_case() {
    let mut seen = std::collections::HashSet::new();
    for k in DiagKind::ALL {
        let id = k.id();
        assert!(
            id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "{id}"
        );
        assert!(seen.insert(id), "duplicate kind id {id}");
    }
}

use super::*;
use crate::{BlockDef, LibraryBlock};

/// Library with `src` (output only), `not` (bool → bool) and `latch`
/// (bool → bool with memory).
fn library() -> Library {
    let mut lib = Library::default();
    for (kind, inputs, memory) in [
        ("src", "", false),
        ("not", "in", false),
        ("latch", "in", true),
    ] {
        let input = if inputs.is_empty() {
            String::new()
        } else {
            format!("[[input]]\nname = \"{inputs}\"\ntype = \"bool\"\n")
        };
        let def: BlockDef = toml::from_str(&format!(
            "[block]\ntype = \"{kind}\"\nname = \"{kind}\"\ngroup = \"Тест\"\nmemory = {memory}\n\
                 {input}[[output]]\nname = \"out\"\ntype = \"bool\"\n"
        ))
        .unwrap();
        lib.insert(LibraryBlock {
            def,
            dir: Default::default(),
            inline_templates: Default::default(),
        });
    }
    lib
}

fn run(blocks: &[(&str, &str)], links: &[(&str, &str)]) -> (Logic, Report) {
    let mut src = String::new();
    for (id, kind) in blocks {
        src += &format!("[[block]]\nid = \"{id}\"\ntype = \"{kind}\"\n");
    }
    for (from, to) in links {
        src += &format!("[[link]]\nfrom = \"{from}\"\nto = \"{to}\"\n");
    }
    let logic: Logic = toml::from_str(&src).unwrap();
    let report = check(&logic, &library(), None);
    (logic, report)
}

fn ids(logic: &Logic, order: &[usize]) -> Vec<String> {
    order.iter().map(|&i| logic.blocks[i].id.clone()).collect()
}

fn bound(var_type: &str, port: &str) -> Report {
    let src = format!(
        "[[var]]
name = \"v\"
type = \"{var_type}\"
             [[block]]
id = \"n\"
type = \"not\"
[block.bind]
{port} = \"v\"
"
    );
    let logic: Logic = toml::from_str(&src).unwrap();
    check(&logic, &library(), None)
}

#[test]
fn bound_input_counts_as_fed() {
    let r = bound("bool", "in");
    assert!(!r.has_errors(), "{:?}", r.diagnostics);
}

#[test]
fn bound_type_mismatch_is_error() {
    assert!(bound("float", "in").has_errors());
}

#[test]
fn param_input_of_unknown_param_is_error() {
    let src = "[[block]]
id = \"n\"
type = \"not\"
param_in = [\"zz\"]
";
    let logic: Logic = toml::from_str(src).unwrap();
    assert!(check(&logic, &library(), None).has_errors());
}

#[test]
fn bound_unknown_var_is_error() {
    let src = "[[block]]
id = \"n\"
type = \"not\"
[block.bind]
in = \"zz\"
";
    let logic: Logic = toml::from_str(src).unwrap();
    assert!(check(&logic, &library(), None).has_errors());
}

#[test]
fn chain_order() {
    let (logic, r) = run(
        &[("c", "not"), ("b", "not"), ("a", "src")],
        &[("a.out", "b.in"), ("b.out", "c.in")],
    );
    assert!(!r.has_errors(), "{:#?}", r.diagnostics);
    assert_eq!(ids(&logic, &r.order), ["a", "b", "c"]);
    assert!(r.feedback.is_empty());
}

#[test]
fn loop_without_memory_is_error() {
    let (_, r) = run(
        &[("x", "not"), ("y", "not")],
        &[("x.out", "y.in"), ("y.out", "x.in")],
    );
    let errors: Vec<_> = r.errors().map(|d| d.message.as_str()).collect();
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert!(
        errors[0].contains("цикл без блока с памятью: x, y"),
        "{errors:#?}"
    );
}

#[test]
fn self_loop_without_memory_is_error() {
    let (_, r) = run(&[("x", "not")], &[("x.out", "x.in")]);
    assert_eq!(r.errors().count(), 1, "{:#?}", r.diagnostics);
}

#[test]
fn downstream_of_loop_is_not_reported_separately() {
    let (_, r) = run(
        &[("x", "not"), ("y", "not"), ("z", "not")],
        &[("x.out", "y.in"), ("y.out", "x.in"), ("y.out", "z.in")],
    );
    assert_eq!(r.errors().count(), 1, "{:#?}", r.diagnostics);
}

#[test]
fn loop_through_memory_is_allowed() {
    // x → m → x: the link into the memory block takes the previous scan's value.
    let (logic, r) = run(
        &[("x", "not"), ("m", "latch")],
        &[("x.out", "m.in"), ("m.out", "x.in")],
    );
    assert!(!r.has_errors(), "{:#?}", r.diagnostics);
    assert_eq!(r.feedback, [0]);
    assert_eq!(ids(&logic, &r.order), ["m", "x"]);
}

/// Library with `src` (bool output) and `timer`, whose input `val` exists only
/// with `source = "input"` and whose required `time` only with `source = "const"`.
/// `wifi` is a single-instance block.
fn conditional_library() -> Library {
    let mut lib = library();
    for src in [
        "[block]\ntype = \"timer\"\nname = \"Таймер\"\ngroup = \"Тест\"\n\
             [[input]]\nname = \"val\"\ntype = \"bool\"\nwhen = 'source == \"input\"'\n\
             [[output]]\nname = \"out\"\ntype = \"bool\"\n\
             [[param]]\nname = \"source\"\ntype = \"string\"\ndefault = \"const\"\n\
             [[param]]\nname = \"time\"\ntype = \"duration\"\nwhen = 'source == \"const\"'\n",
        "[block]\ntype = \"wifi\"\nname = \"Wi-Fi\"\ngroup = \"Тест\"\nsingle = true\n",
    ] {
        let def: BlockDef = toml::from_str(src).unwrap();
        assert_eq!(def.validate(&def.info.kind), Ok(()));
        lib.insert(LibraryBlock {
            def,
            dir: Default::default(),
            inline_templates: Default::default(),
        });
    }
    lib
}

fn check_src(src: &str) -> Vec<String> {
    let logic: Logic = toml::from_str(src).unwrap();
    check(&logic, &conditional_library(), None)
        .errors()
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn disabled_port_and_param() {
    // Default source "const": `time` is required, `val` does not exist.
    let errors = check_src(
        "[[block]]\nid = \"a\"\ntype = \"src\"\n\
             [[block]]\nid = \"t\"\ntype = \"timer\"\n\
             [[link]]\nfrom = \"a.out\"\nto = \"t.val\"\n",
    );
    assert_eq!(errors.len(), 2, "{errors:#?}");
    assert!(errors
        .iter()
        .any(|e| e.contains("не задан параметр «time»")));
    assert!(errors.iter().any(|e| e.contains("отключён параметрами")));

    // Source "input": `val` must be connected, `time` is not needed.
    let errors = check_src("[[block]]\nid = \"t\"\ntype = \"timer\"\nsource = \"input\"\n");
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert!(errors[0].contains("на вход «val» ничего не подано"));
    let errors = check_src(
        "[[block]]\nid = \"a\"\ntype = \"src\"\n\
             [[block]]\nid = \"t\"\ntype = \"timer\"\nsource = \"input\"\n\
             [[link]]\nfrom = \"a.out\"\nto = \"t.val\"\n",
    );
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn broken_condition_value_is_reported() {
    // Adding a number to a string fails at evaluation.
    let mut lib = conditional_library();
    let def: BlockDef = toml::from_str(
        "[block]\ntype = \"odd\"\nname = \"odd\"\ngroup = \"Тест\"\n\
             [[output]]\nname = \"q\"\ntype = \"bool\"\nwhen = \"m + 1 > 0\"\n\
             [[param]]\nname = \"m\"\ntype = \"string\"\ndefault = \"x\"\n",
    )
    .unwrap();
    lib.insert(LibraryBlock {
        def,
        dir: Default::default(),
        inline_templates: Default::default(),
    });
    let logic: Logic = toml::from_str("[[block]]\nid = \"o\"\ntype = \"odd\"\n").unwrap();
    let r = check(&logic, &lib, None);
    assert_eq!(r.errors().count(), 1, "{:#?}", r.diagnostics);
}

#[test]
fn single_block() {
    let errors = check_src("[[block]]\nid = \"w\"\ntype = \"wifi\"\n");
    assert!(errors.is_empty(), "{errors:#?}");
    let errors = check_src(
        "[[block]]\nid = \"w1\"\ntype = \"wifi\"\n\
             [[block]]\nid = \"w2\"\ntype = \"wifi\"\n\
             [[block]]\nid = \"w3\"\ntype = \"wifi\"\n",
    );
    assert_eq!(errors.len(), 2, "{errors:#?}");
    assert!(errors[0].contains("«w2»") && errors[0].contains("уже есть «w1»"));
    assert!(errors[1].contains("«w3»") && errors[1].contains("уже есть «w1»"));
}

#[test]
fn memory_block_outside_loop_is_not_feedback() {
    let (_, r) = run(&[("a", "src"), ("m", "latch")], &[("a.out", "m.in")]);
    assert!(!r.has_errors(), "{:#?}", r.diagnostics);
    assert!(r.feedback.is_empty());
}

/// Library with `out` (pin for output), `ain` (ADC pin), `bus` (shared pin)
/// and `uart` (two pins, -1 means "not used").
fn pin_library() -> Library {
    let mut lib = Library::default();
    for src in [
        "[block]
type = \"out\"
name = \"o\"
group = \"Тест\"
             [[param]]
name = \"pin\"
type = \"pin\"
mode = \"out\"
",
        "[block]
type = \"ain\"
name = \"a\"
group = \"Тест\"
             [[param]]
name = \"pin\"
type = \"pin\"
mode = \"adc\"
",
        "[block]
type = \"pwm\"
name = \"p\"
group = \"Тест\"
             [[param]]
name = \"pin\"
type = \"pin\"
mode = \"pwm\"
",
        "[block]
type = \"din\"
name = \"d\"
group = \"Тест\"
             [[param]]
name = \"pin\"
type = \"pin\"
mode = \"in\"
",
        "[block]
type = \"bus\"
name = \"b\"
group = \"Тест\"
             [[param]]
name = \"pin\"
type = \"pin\"
shared = true
",
        "[block]
type = \"uart\"
name = \"u\"
group = \"Тест\"
             [[param]]
name = \"rx\"
type = \"pin\"
min = -1
default = -1
             [[param]]
name = \"tx\"
type = \"pin\"
min = -1
default = -1
",
    ] {
        let def: BlockDef = toml::from_str(src).unwrap();
        lib.insert(LibraryBlock {
            def,
            dir: Default::default(),
            inline_templates: Default::default(),
        });
    }
    lib
}

fn pin_project(blocks: &[(&str, &str, &str)]) -> Project {
    let mut src = String::new();
    for (id, kind, params) in blocks {
        src += &format!(
            "[[block]]
id = \"{id}\"
type = \"{kind}\"
{params}
"
        );
    }
    Project {
        path: Default::default(),
        config: toml::from_str(
            "[project]
name = \"p\"
[board]
fqbn = \"esp32:esp32:esp32\"
",
        )
        .unwrap(),
        logic: toml::from_str(&src).unwrap(),
        layout: Default::default(),
    }
}

fn pin_messages(blocks: &[(&str, &str, &str)]) -> Vec<String> {
    check_project(&pin_project(blocks), &pin_library())
        .diagnostics
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn pin_used_twice() {
    let m = pin_messages(&[
        ("a", "out", "pin = 4"),
        ("b", "out", "pin = 4"),
        ("c", "out", "pin = 5"),
    ]);
    assert_eq!(m, ["блок «b»: вывод GPIO4 уже занят блоком «a»"]);
    let m = pin_messages(&[(
        "u",
        "uart",
        "rx = 16
tx = 16",
    )]);
    assert_eq!(
        m,
        ["блок «u»: параметры «rx» и «tx» на одном выводе GPIO16"]
    );
}

#[test]
fn shared_pin_and_unused_pins() {
    let m = pin_messages(&[
        ("t1", "bus", "pin = 4"),
        ("t2", "bus", "pin = 4"),
        ("u1", "uart", ""),
        ("u2", "uart", ""),
    ]);
    assert!(m.is_empty(), "{m:#?}");
    let m = pin_messages(&[("t1", "bus", "pin = 4"), ("o", "out", "pin = 4")]);
    assert_eq!(m, ["блок «o»: вывод GPIO4 уже занят блоком «t1»"]);
}

#[test]
fn pins_of_the_board() {
    let m = pin_messages(&[
        ("a", "out", "pin = 6"),
        ("b", "out", "pin = 34"),
        ("c", "ain", "pin = 5"),
        ("d", "out", "pin = 40"),
        ("e", "out", "pin = 1"),
        ("f", "ain", "pin = 36"),
    ]);
    assert_eq!(
        m,
        [
            "блок «a»: вывод GPIO6 занят флеш-памятью у ESP32",
            "блок «b»: вывод GPIO34 работает только на вход у ESP32",
            "блок «c»: вывод GPIO5 без АЦП у ESP32",
            "блок «d»: у ESP32 нет вывода GPIO40",
            "блок «e»: вывод GPIO1 — UART0: заливка и отладка по USB",
        ]
    );
}

#[test]
fn pins_of_a_user_board() {
    use crate::boards::{BoardDef, BoardInfo, BoardPin, Boards, UserBoard};
    let pin = |gpio, name: &str, busy: Option<&str>| BoardPin {
        gpio,
        name: name.into(),
        note: String::new(),
        busy: busy.map(Into::into),
        functions: None,
    };
    let def = BoardDef {
        info: BoardInfo {
            id: "mine".into(),
            name: "Моя плата".into(),
            chip: "esp32".into(),
            fqbn: "esp32:esp32:esp32".into(),
            version: 1,
            options: Default::default(),
        },
        pins: vec![
            pin(4, "OUT1", None),
            pin(36, "A1", None),
            pin(15, "SCL", Some("шина I2C")),
        ],
        expander_pins: Vec::new(),
        devices: Vec::new(),
    };
    let mut boards = Boards::default();
    boards.insert(UserBoard {
        def,
        dir: Default::default(),
    });
    let mut lib = pin_library();
    lib.set_boards(boards);
    let mut project = pin_project(&[
        ("a", "out", "pin = 4"),
        // The chip has GPIO5, the board does not bring it out.
        ("b", "out", "pin = 5"),
        ("c", "ain", "pin = 36"),
        ("d", "out", "pin = 15"),
    ]);
    project.config.board.model = Some("mine".into());
    let m: Vec<String> = check_project(&project, &lib)
        .diagnostics
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert_eq!(
        m,
        [
            "блок «b»: у Моя плата нет вывода GPIO5",
            "блок «d»: вывод GPIO15 — шина I2C",
        ]
    );
    // Without the model the same schema is checked against the chip table.
    project.config.board.model = None;
    assert!(!check_project(&project, &lib)
        .diagnostics
        .iter()
        .any(|d| d.message.contains("Моя плата")));
}

#[test]
fn a_pin_without_the_needed_function_is_refused() {
    use crate::boards::{BoardDef, BoardInfo, BoardPin, Boards, UserBoard};
    use crate::PinFunction::*;
    let pin = |gpio, functions: Option<Vec<_>>| BoardPin {
        gpio,
        name: String::new(),
        note: String::new(),
        busy: None,
        functions,
    };
    let mut boards = Boards::default();
    boards.insert(UserBoard {
        def: BoardDef {
            info: BoardInfo {
                id: "mine".into(),
                name: "Моя плата".into(),
                chip: "esp32".into(),
                fqbn: "esp32:esp32:esp32".into(),
                version: 1,
                options: Default::default(),
            },
            pins: vec![
                pin(4, Some(vec![Dout])),
                pin(5, Some(vec![Din, Pwm])),
                pin(36, Some(vec![Adc])),
                pin(13, None),
            ],
            expander_pins: Vec::new(),
            devices: Vec::new(),
        },
        dir: Default::default(),
    });
    let mut lib = pin_library();
    lib.set_boards(boards);
    let mut project = pin_project(&[
        ("ok_out", "out", "pin = 4"),
        ("ok_pwm", "pwm", "pin = 5"),
        ("ok_adc", "ain", "pin = 36"),
        ("ok_any", "din", "pin = 13"),
        // GPIO4 has an ADC on the chip, but the board gives it only as an output
        ("no_adc", "ain", "pin = 4"),
        ("no_pwm", "pwm", "pin = 4"),
        ("no_in", "din", "pin = 4"),
        ("no_out", "out", "pin = 5"),
    ]);
    project.config.board.model = Some("mine".into());
    let m: Vec<String> = check_project(&project, &lib)
        .diagnostics
        .into_iter()
        .map(|d| d.message)
        .filter(|m| m.contains("у Моя плата"))
        .collect();
    assert_eq!(
        m,
        [
            "блок «no_adc»: вывод GPIO4 без АЦП у Моя плата",
            "блок «no_pwm»: вывод GPIO4 не умеет ШИМ у Моя плата",
            "блок «no_in»: вывод GPIO4 не работает как цифровой вход у Моя плата",
            "блок «no_out»: вывод GPIO5 не работает как цифровой выход у Моя плата",
        ]
    );
}

#[test]
fn a_block_of_another_board() {
    use crate::boards::{BoardDef, BoardInfo, Boards, UserBoard};
    let mut boards = Boards::default();
    boards.insert(UserBoard {
        def: BoardDef {
            info: BoardInfo {
                id: "mine".into(),
                name: "Моя плата".into(),
                chip: "esp32".into(),
                fqbn: "esp32:esp32:esp32".into(),
                version: 1,
                options: Default::default(),
            },
            pins: Vec::new(),
            expander_pins: Vec::new(),
            devices: Vec::new(),
        },
        dir: Default::default(),
    });
    let mut lib = pin_library();
    lib.set_boards(boards);
    let mut relay = lib.get("out").unwrap().clone();
    relay.def.info.kind = "mine_out".into();
    relay.def.info.board = Some("mine".into());
    lib.insert(relay);

    let mut project = pin_project(&[("r", "mine_out", "pin = 4")]);
    // A project of a built-in board, or of no board, cannot use it.
    for model in [None, Some("other")] {
        project.config.board.model = model.map(Into::into);
        let report = check_project(&project, &lib);
        let kinds: Vec<_> = report.errors().map(|d| d.kind).collect();
        assert_eq!(kinds, [DiagKind::BoardBlock], "{:?}", report.diagnostics);
        let message = &report.diagnostics[0].message;
        assert!(message.contains("для платы «Моя плата»"), "{message}");
    }
    project.config.board.model = Some("mine".into());
    let report = check_project(&project, &lib);
    assert!(!report.has_errors(), "{:?}", report.diagnostics);
}

#[test]
fn the_board_does_not_warn_its_own_devices_about_its_pins() {
    use crate::boards::{BoardDef, BoardInfo, BoardPin, Boards, UserBoard};
    let mut boards = Boards::default();
    boards.insert(UserBoard {
        def: BoardDef {
            info: BoardInfo {
                id: "mine".into(),
                name: "Моя плата".into(),
                chip: "esp32".into(),
                fqbn: "esp32:esp32:esp32".into(),
                version: 1,
                options: Default::default(),
            },
            pins: vec![BoardPin {
                gpio: 4,
                name: "SDA".into(),
                note: String::new(),
                busy: Some("шина I2C".into()),
                functions: None,
            }],
            expander_pins: Vec::new(),
            devices: Vec::new(),
        },
        dir: Default::default(),
    });
    let mut lib = pin_library();
    lib.set_boards(boards);
    let mut relay = lib.get("out").unwrap().clone();
    relay.def.info.kind = "mine_out".into();
    relay.def.info.board = Some("mine".into());
    lib.insert(relay);

    let mut project = pin_project(&[("r", "mine_out", "pin = 4"), ("o", "out", "pin = 4")]);
    project.config.board.model = Some("mine".into());
    let m: Vec<String> = check_project(&project, &lib)
        .diagnostics
        .into_iter()
        .map(|d| d.message)
        .collect();
    // Only the ordinary block hears about the bus (and about sharing the pin).
    assert!(
        m.iter()
            .any(|s| s.contains("«o»") && s.contains("шина I2C")),
        "{m:?}"
    );
    assert!(
        !m.iter()
            .any(|s| s.contains("«r»") && s.contains("шина I2C")),
        "{m:?}"
    );
}

fn retain_fast_report(retain: bool, fast: bool) -> Report {
    let mut lib = library();
    let def: BlockDef = toml::from_str(&format!(
        "[block]\ntype = \"meas\"\nname = \"meas\"\ngroup = \"Тест\"\n\
         [[output]]\nname = \"out\"\ntype = \"float\"\nfast = {fast}\n"
    ))
    .unwrap();
    lib.insert(LibraryBlock {
        def,
        dir: Default::default(),
        inline_templates: Default::default(),
    });
    let src = format!(
        "[[var]]\nname = \"v\"\ntype = \"float\"\nretain = {retain}\n\
         [[block]]\nid = \"m\"\ntype = \"meas\"\n\
         [[block]]\nid = \"s\"\ntype = \"var_set\"\nvar = \"v\"\n\
         [[link]]\nfrom = \"m.out\"\nto = \"s.in\"\n"
    );
    let logic: Logic = toml::from_str(&src).unwrap();
    check(&logic, &lib, None)
}

fn has_retain_fast(r: &Report) -> bool {
    r.diagnostics
        .iter()
        .any(|d| d.kind == DiagKind::RetainFast && d.severity == Severity::Warning)
}

#[test]
fn retain_var_fed_from_fast_output_warns() {
    let r = retain_fast_report(true, true);
    assert!(!r.has_errors(), "{:?}", r.diagnostics);
    assert!(has_retain_fast(&r), "{:?}", r.diagnostics);
}

#[test]
fn retain_fast_not_reported_for_slow_output_or_plain_var() {
    assert!(!has_retain_fast(&retain_fast_report(true, false)));
    assert!(!has_retain_fast(&retain_fast_report(false, true)));
}

#[test]
fn retain_config_round_trip_has_no_extra_fields() {
    use crate::project::{ProjectConfig, RetainConfig};
    let base = "[project]\nname = \"p\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n";
    let cfg: ProjectConfig = toml::from_str(base).unwrap();
    assert_eq!(cfg.retain, RetainConfig::default());
    let text = toml::to_string(&cfg).unwrap();
    assert!(!text.contains("retain"), "{text}");
    let custom: ProjectConfig =
        toml::from_str(&format!("{base}\n[retain]\nperiod_s = 0\n")).unwrap();
    assert_eq!((custom.retain.settle_s, custom.retain.period_s), (5, 0));
    let back: ProjectConfig = toml::from_str(&toml::to_string(&custom).unwrap()).unwrap();
    assert_eq!(back.retain, custom.retain);
}

#[test]
fn var_timing_override_and_default_serialization() {
    use crate::vars::Var;
    let var: Var = toml::from_str("name = \"v\"\ntype = \"int\"\n").unwrap();
    assert_eq!((var.settle_s, var.period_s), (None, None));
    assert!(!toml::to_string(&var).unwrap().contains("_s"));
    let own: Var =
        toml::from_str("name = \"v\"\ntype = \"int\"\nsettle_s = 1\nperiod_s = 0\n").unwrap();
    assert_eq!((own.settle_s, own.period_s), (Some(1), Some(0)));
}
