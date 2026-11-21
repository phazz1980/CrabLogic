//! The command set against an in-memory project.

use std::fs;
use std::path::PathBuf;

use crab_ai::{call, tools, Context};
use crab_core::{Library, Project};
use serde_json::{json, Value};

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

struct Mem {
    project: Project,
    lib: Library,
    lib_dir: PathBuf,
    edits: usize,
}

impl Context for Mem {
    fn project(&self) -> &Project {
        &self.project
    }

    fn library(&self) -> &Library {
        &self.lib
    }

    fn library_dir(&self) -> PathBuf {
        self.lib_dir.clone()
    }

    fn edit(
        &mut self,
        edit: &mut dyn FnMut(&mut Project) -> Result<(), String>,
    ) -> Result<(), String> {
        // All or nothing, as in the real contexts.
        let mut p = self.project.clone();
        edit(&mut p)?;
        self.project = p;
        self.edits += 1;
        Ok(())
    }

    fn upload_modules(&mut self, _: Option<String>, _: bool) -> Result<String, String> {
        Err("заливка в тестах не нужна".into())
    }

    fn live_values(&mut self) -> Result<Vec<(String, String, bool)>, String> {
        Ok(vec![("button.out".into(), "1".into(), false)])
    }

    fn reload_library(&mut self) -> Result<(), String> {
        let mut lib = Library::load(repo_path("library")).unwrap().0;
        lib.add_user_dir(&self.lib_dir).map_err(|e| e.to_string())?;
        self.lib = lib;
        Ok(())
    }
}

fn ctx(name: &str) -> Mem {
    let project = Project::load(repo_path("examples/button")).unwrap();
    let lib_dir = repo_path("target/test-ai-lib").join(name);
    let _ = fs::remove_dir_all(&lib_dir);
    fs::create_dir_all(&lib_dir).unwrap();
    Mem {
        project,
        lib: Library::load(repo_path("library")).unwrap().0,
        lib_dir,
        edits: 0,
    }
}

fn run(c: &mut Mem, name: &str, args: Value) -> Result<Value, String> {
    call(c, name, &args)
}

#[test]
fn every_tool_has_an_object_schema() {
    for t in tools() {
        assert_eq!(t.input_schema["type"], "object", "{}", t.name);
        assert!(!t.description.is_empty());
    }
}

#[test]
fn builds_a_schema_step_by_step() {
    let mut c = ctx("steps");
    let r = run(
        &mut c,
        "add_block",
        json!({"type": "timer_on", "id": "delay", "params": {"time": "2s"}}),
    )
    .unwrap();
    assert_eq!(r["id"], "delay");
    // The new timer's input is not connected yet: the check says so.
    assert!(r["check"]["errors"].to_string().contains("delay"));
    assert_eq!(r["check"]["errors"][0]["kind"], "input-empty");

    run(
        &mut c,
        "connect",
        json!({"from": "button.out", "to": "delay.in"}),
    )
    .unwrap();
    let r = run(
        &mut c,
        "connect",
        json!({"from": "delay.out", "to": "led.in"}),
    )
    .unwrap();
    assert_eq!(r["check"]["errors"], json!([]));
    assert_eq!(r["check"]["order"], json!(["button", "delay", "led"]));

    let schema = run(&mut c, "get_schema", json!({})).unwrap();
    let blocks = schema["logic"]["block"].as_array().unwrap();
    assert_eq!(blocks.len(), 3);
    assert_eq!(schema["placement"]["delay"]["sheet"], 1);

    let r = run(&mut c, "build", json!({})).unwrap();
    let modules = r["modules"].as_array().unwrap();
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0]["sheet"], 1);
    assert!(modules[0]["wasm_bytes"].as_u64().unwrap() > 0);
}

#[test]
fn failed_edits_change_nothing() {
    let mut c = ctx("failed");
    let before = c.project.logic.clone();
    let err = run(
        &mut c,
        "add_block",
        json!({"type": "timer_on", "params": {"time": "скоро"}}),
    )
    .unwrap_err();
    assert!(err.contains("time"), "{err}");
    assert!(run(
        &mut c,
        "connect",
        json!({"from": "led.in", "to": "button.out"})
    )
    .is_err());
    assert!(run(&mut c, "connect", json!({"from": "nope", "to": "led.in"})).is_err());
    assert_eq!(c.project.logic, before);
    assert_eq!(c.edits, 0);
}

#[test]
fn constants_and_inversion() {
    let mut c = ctx("const");
    run(
        &mut c,
        "invert_port",
        json!({"port": "button.out", "output": true}),
    )
    .unwrap();
    let schema = run(&mut c, "get_schema", json!({})).unwrap();
    assert!(schema["logic"].to_string().contains("\"invert\":[\"out\"]"));
    let r = run(&mut c, "build", json!({})).unwrap();
    assert!(!r["modules"].as_array().unwrap().is_empty());
    // Not an input of the button, and not a logical constant.
    assert!(run(
        &mut c,
        "invert_port",
        json!({"port": "button.out", "output": false})
    )
    .is_err());
    assert!(run(
        &mut c,
        "set_input_const",
        json!({"input": "led.in", "value": 1.5})
    )
    .is_err());

    // A constant replaces the link; removing it leaves the input unconnected.
    let r = run(
        &mut c,
        "set_input_const",
        json!({"input": "led.in", "value": true}),
    )
    .unwrap();
    assert_eq!(r["check"]["errors"], json!([]));
    assert!(c.project.logic.links.is_empty());
    let r = run(
        &mut c,
        "set_input_const",
        json!({"input": "led.in", "value": null}),
    )
    .unwrap();
    assert!(r["check"]["errors"].to_string().contains("led"));
}

#[test]
fn variables() {
    let mut c = ctx("vars");
    run(
        &mut c,
        "set_variable",
        json!({"name": "limit", "type": "float", "init": 22.5, "description": "уставка"}),
    )
    .unwrap();
    let r = run(
        &mut c,
        "add_variable_block",
        json!({"var": "limit", "write": false}),
    )
    .unwrap();
    assert_eq!(r["id"], "limit_r1");
    run(
        &mut c,
        "set_variable",
        json!({"name": "limit", "type": "float", "rename_to": "setpoint"}),
    )
    .unwrap();
    assert_eq!(
        c.project.logic.blocks.last().unwrap().var_name(),
        Some("setpoint")
    );
    run(&mut c, "remove_variable", json!({"name": "setpoint"})).unwrap();
    assert!(c.project.logic.vars.is_empty());
}

#[test]
fn expanders() {
    let mut c = ctx("expanders");
    run(
        &mut c,
        "set_expander",
        json!({"id": "relays", "chip": "pcf8575"}),
    )
    .unwrap();
    // an address of another chip is refused, and the project stays as it was
    let bad = run(
        &mut c,
        "set_expander",
        json!({"id": "relays", "address": 56}),
    );
    assert!(bad.unwrap_err().contains("0x38"));
    for id in ["lamp", "fan"] {
        run(
            &mut c,
            "add_block",
            json!({"type": "digital_out", "id": id}),
        )
        .unwrap();
    }
    run(
        &mut c,
        "place_on_expander",
        json!({"block": "lamp", "expander": "relays", "pin": 12}),
    )
    .unwrap();
    // the schema tells which pins are taken
    let schema = run(&mut c, "get_schema", json!({})).unwrap();
    let e = &schema["expanders"][0];
    assert_eq!(e["id"], "relays");
    assert_eq!(e["pins"], 16);
    assert_eq!(e["taken"][0], json!({"pin": 12, "block": "lamp"}));
    // busy, out of range and still-used expander are refused
    let busy = run(
        &mut c,
        "place_on_expander",
        json!({"block": "fan", "expander": "relays", "pin": 12}),
    );
    assert!(busy.unwrap_err().contains("lamp"));
    let range = run(
        &mut c,
        "place_on_expander",
        json!({"block": "fan", "expander": "relays", "pin": 16}),
    );
    assert!(range.unwrap_err().contains("P15"));
    let used = run(&mut c, "remove_expander", json!({"id": "relays"}));
    assert!(used.unwrap_err().contains("lamp"));
    // a rename takes the blocks along; taking them off allows the removal
    run(
        &mut c,
        "set_expander",
        json!({"id": "relays", "rename_to": "lamps"}),
    )
    .unwrap();
    assert_eq!(
        c.project.logic.block("lamp").unwrap().expander.as_deref(),
        Some("lamps")
    );
    run(
        &mut c,
        "place_on_expander",
        json!({"block": "lamp", "expander": null}),
    )
    .unwrap();
    run(&mut c, "remove_expander", json!({"id": "lamps"})).unwrap();
    assert!(c.project.logic.expanders.is_empty());
}

#[test]
fn library_queries() {
    let mut c = ctx("lib");
    let found = run(&mut c, "list_block_types", json!({"filter": "таймер"})).unwrap();
    let kinds: Vec<&str> = found
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["type"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"timer_on"), "{kinds:?}");
    let t = run(&mut c, "get_block_type", json!({"type": "timer_on"})).unwrap();
    assert!(t["docs"].as_str().unwrap().contains("TON"));
    // Built-in blocks run from Rust code of the program: no module file of their own.
    assert_eq!(t["has_wasm"], false);
}

/// Standard base64 of `data` (the tool takes `block.wasm` as text).
fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
        out.extend(std::iter::repeat_n('=', 3 - chunk.len()));
    }
    out
}

#[test]
fn creates_block_types_only_when_valid() {
    let mut c = ctx("create");
    let example =
        repo_path("skills/crablogic-block/examples/window_check/project/library/window_check");
    let toml = fs::read_to_string(example.join("block.toml")).unwrap();
    let wasm = base64(&fs::read(example.join("block.wasm")).unwrap());

    // The type in block.toml must match the requested one.
    let bad = run(
        &mut c,
        "create_block_type",
        json!({"type": "twice", "block_toml": toml, "wasm_base64": wasm}),
    );
    assert!(bad.is_err());
    assert!(!c.lib_dir.join("twice").exists());

    // A block without a module (the old C++ way) is refused.
    let none = run(
        &mut c,
        "create_block_type",
        json!({"type": "window_check", "block_toml": toml, "wasm_base64": "", "template": "// @loop\n"}),
    );
    assert!(none.is_err());
    assert!(!c.lib_dir.join("window_check").exists());
    let junk = run(
        &mut c,
        "create_block_type",
        json!({"type": "window_check", "block_toml": toml, "wasm_base64": "это не base64"}),
    );
    assert!(junk.unwrap_err().contains("base64"));

    run(
        &mut c,
        "create_block_type",
        json!({"type": "window_check", "block_toml": toml, "wasm_base64": wasm,
               "docs": "# Окно\n"}),
    )
    .unwrap();
    assert!(c.lib.get("window_check").is_some() && c.lib.is_user("window_check"));
    let t = run(&mut c, "get_block_type", json!({"type": "window_check"})).unwrap();
    assert_eq!(t["has_wasm"], true);
    let builtin = "[block]
type = \"and\"
name = \"И\"
group = \"Логика\"
";
    let taken = run(
        &mut c,
        "create_block_type",
        json!({"type": "and", "block_toml": builtin, "wasm_base64": wasm}),
    );
    assert!(taken.unwrap_err().contains("встроенным"));
    let again = run(
        &mut c,
        "create_block_type",
        json!({"type": "window_check", "block_toml": toml, "wasm_base64": wasm}),
    );
    assert!(again.unwrap_err().contains("уже есть"));
}

#[test]
fn live_values_and_unknown_tool() {
    let mut c = ctx("live");
    let v = run(&mut c, "read_live_values", json!({})).unwrap();
    assert_eq!(v[0]["name"], "button.out");
    assert!(run(&mut c, "fly", json!({})).is_err());
}

#[test]
fn moves_a_block_and_reports_the_ports_in_view() {
    let mut c = ctx("move");
    run(
        &mut c,
        "add_block",
        json!({"type": "timer_on", "id": "delay"}),
    )
    .unwrap();
    run(
        &mut c,
        "move_block",
        json!({"id": "delay", "x": 300.0, "y": 120.0}),
    )
    .unwrap();
    let schema = run(&mut c, "get_schema", json!({})).unwrap();
    assert_eq!(schema["placement"]["delay"]["x"], 300.0);
    assert_eq!(schema["view"]["delay"]["inputs"][0]["name"], "in");
    assert!(run(&mut c, "move_block", json!({"id": "nope", "x": 1, "y": 1})).is_err());
}

#[test]
fn parameter_becomes_an_input() {
    let mut c = ctx("param_input");
    run(&mut c, "add_block", json!({"type": "timer_on", "id": "t"})).unwrap();
    // Pins and unknown parameters are not inputs.
    assert!(run(
        &mut c,
        "set_param_input",
        json!({"block": "t", "param": "nope"})
    )
    .is_err());
    run(
        &mut c,
        "set_param_input",
        json!({"block": "t", "param": "time"}),
    )
    .unwrap();
    run(
        &mut c,
        "set_input_const",
        json!({"input": "t.time", "value": 750}),
    )
    .unwrap();
    run(
        &mut c,
        "set_input_const",
        json!({"input": "t.in", "value": true}),
    )
    .unwrap();
    let r = run(&mut c, "build", json!({})).unwrap();
    assert!(!r["modules"].as_array().unwrap().is_empty());
    run(
        &mut c,
        "set_param_input",
        json!({"block": "t", "param": "time", "on": false}),
    )
    .unwrap();
    assert!(c.project.logic.block("t").unwrap().param_in.is_empty());
}

#[test]
fn panel_commands_build_the_panel_step_by_step() {
    let mut c = ctx("panel_steps");
    // An empty panel: nothing bound, the free inputs and outputs of the project are listed.
    let r = run(&mut c, "get_panel", json!({})).unwrap();
    assert_eq!(r["devices"].as_array().unwrap().len(), 0);
    let free: Vec<&str> = r["free_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        free.contains(&"button") && free.contains(&"led"),
        "{free:?}"
    );
    // Designations are given by the kind.
    let a = run(
        &mut c,
        "add_panel_device",
        json!({"kind": "contactor", "bind": "led", "load": "M1"}),
    )
    .unwrap();
    assert_eq!(a["id"], "K1");
    run(&mut c, "add_panel_device", json!({"kind": "motor"})).unwrap();
    run(
        &mut c,
        "add_panel_device",
        json!({"kind": "button", "id": "S1", "bind": "button"}),
    )
    .unwrap();
    assert_eq!(c.project.panel.devices.len(), 3);
    assert_eq!(c.project.panel.device("K1").unwrap().bind, "led");
    // Mistakes are refused and change nothing.
    let edits = c.edits;
    assert!(run(
        &mut c,
        "add_panel_device",
        json!({"kind": "motor", "id": "K1"})
    )
    .is_err());
    assert!(run(
        &mut c,
        "add_panel_device",
        json!({"kind": "motor", "id": "a b"})
    )
    .is_err());
    assert!(run(&mut c, "add_panel_device", json!({"kind": "teapot"})).is_err());
    assert!(run(
        &mut c,
        "set_panel_device",
        json!({"id": "Z9", "rating": "1"})
    )
    .is_err());
    assert_eq!(c.edits, edits);
    // A wrong binding is not refused (the panel is drawn while unfinished) but the check says so.
    let r = run(
        &mut c,
        "set_panel_device",
        json!({"id": "S1", "bind": "led"}),
    )
    .unwrap();
    assert!(!r["check"]["errors"].as_array().unwrap().is_empty());
    let r = run(
        &mut c,
        "set_panel_device",
        json!({"id": "S1", "bind": "button", "rating": "10 A"}),
    )
    .unwrap();
    assert!(r["check"]["errors"].as_array().unwrap().is_empty(), "{r}");
    // Renaming: what the device switches follows it.
    run(
        &mut c,
        "set_panel_device",
        json!({"id": "M1", "new_id": "M2"}),
    )
    .unwrap();
    assert_eq!(c.project.panel.device("K1").unwrap().load, "M2");
    // null clears.
    run(
        &mut c,
        "set_panel_device",
        json!({"id": "S1", "rating": null}),
    )
    .unwrap();
    assert_eq!(c.project.panel.device("S1").unwrap().rating, "");
    // Removing clears the links to the device.
    run(&mut c, "remove_panel_device", json!({"id": "M2"})).unwrap();
    assert_eq!(c.project.panel.device("K1").unwrap().load, "");
    assert!(run(&mut c, "remove_panel_device", json!({"id": "M2"})).is_err());
    // Supply.
    run(&mut c, "set_panel_supply", json!({"supply": "ac400"})).unwrap();
    assert_eq!(c.project.panel.supply, crab_core::Supply::Ac400);
    assert!(run(&mut c, "set_panel_supply", json!({"supply": "110v"})).is_err());
    // The schema is untouched by all this.
    assert_eq!(
        c.project.logic,
        Project::load(repo_path("examples/button")).unwrap().logic
    );
}

#[test]
fn building_the_panel_needs_devices_and_writes_a_pdf() {
    let mut c = ctx("panel_build");
    let out = c.lib_dir.join("щит.pdf");
    let args = json!({"out": out.to_str().unwrap(), "lang": "en", "format": "a4"});
    assert!(run(&mut c, "build_panel", args.clone())
        .unwrap_err()
        .contains("щит пуст"));
    for (kind, id, load) in [
        ("feed", "X1", ""),
        ("breaker", "Q1", "M1"),
        ("motor", "M1", ""),
    ] {
        run(
            &mut c,
            "add_panel_device",
            json!({"kind": kind, "id": id, "load": load}),
        )
        .unwrap();
    }
    assert!(run(&mut c, "build_panel", json!({"lang": "de"})).is_err());
    assert!(run(&mut c, "build_panel", json!({"format": "a0"})).is_err());
    // The PDF needs the `crablogic-pdf` helper; without it the command says why.
    match run(&mut c, "build_panel", args) {
        Ok(r) => {
            assert!(r["sheets"].as_u64().unwrap() >= 3);
            assert!(fs::metadata(&out).unwrap().len() > 1000);
            // The numbers of the wires are kept in the project.
            assert!(!c.project.panel.wire_numbers.is_empty());
        }
        Err(e) => assert!(!e.is_empty()),
    }
}

#[test]
fn panel_commands_are_offered_to_the_model() {
    let names: Vec<&str> = tools().iter().map(|t| t.name).collect();
    for n in crab_ai::panel::NAMES {
        assert!(names.contains(&n), "{n}");
    }
}

fn has_link(c: &Mem, from: &str, to: &str) -> bool {
    c.project
        .logic
        .links
        .iter()
        .any(|l| l.from.to_string() == from && l.to.to_string() == to)
}

#[test]
fn a_new_block_goes_into_a_link_in_one_edit() {
    let mut c = ctx("insert");
    let r = run(
        &mut c,
        "insert_block_in_link",
        json!({"to": "led.in", "type": "not"}),
    )
    .unwrap();
    assert_eq!(r["result"], "spliced");
    let id = r["id"].as_str().unwrap().to_owned();
    assert_eq!(c.edits, 1);
    assert!(!has_link(&c, "button.out", "led.in"));
    assert!(has_link(&c, "button.out", &format!("{id}.in")));
    assert!(has_link(&c, &format!("{id}.out"), "led.in"));
}

#[test]
fn a_block_of_the_schema_goes_into_a_link_and_a_branch_is_reported() {
    let mut c = ctx("insert-existing");
    run(
        &mut c,
        "add_block",
        json!({"type": "digital_out", "id": "lamp"}),
    )
    .unwrap();
    let r = run(
        &mut c,
        "insert_block_in_link",
        json!({"to": "led.in", "block": "lamp"}),
    )
    .unwrap();
    assert_eq!(r["result"], "branch");
    assert!(has_link(&c, "button.out", "led.in"));
    assert!(has_link(&c, "button.out", "lamp.in"));
}

#[test]
fn a_refused_insertion_changes_nothing() {
    let mut c = ctx("insert-refused");
    let before = c.project.clone();
    let err = run(
        &mut c,
        "insert_block_in_link",
        json!({"to": "led.en", "type": "not"}),
    )
    .unwrap_err();
    assert!(err.contains("led.en"), "{err}");
    assert_eq!(c.project, before);
    // Exactly one of `block` and `type`.
    assert!(run(&mut c, "insert_block_in_link", json!({"to": "led.in"})).is_err());
}
