//! Sheets as modules: the border between sheets (tags) and the errors.

use std::path::PathBuf;

use crab_codegen::{Program, SheetError, TagKind};
use crab_core::{Library, ParamValue, Project};

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

fn background() -> Project {
    Project::load(repo_path("examples/background")).unwrap()
}

#[test]
fn border_of_two_sheets() {
    let lib = library();
    let mut project = background();
    project.logic.vars[0].retain = false;
    let program = Program::build(&project, &lib).unwrap();
    assert_eq!(program.sheets(), vec![1, 2]);

    // The variable (used on both sheets, written on 2) comes first, then press.out.
    let table = program.tag_table();
    assert_eq!(table.len(), 2);
    assert_eq!(table[0].kind, TagKind::Var("Нажатий всего".into()));
    assert_eq!(table[0].owner, 2);
    assert_eq!(table[1].kind, TagKind::Signal("press.out".into()));
    assert_eq!(table[1].owner, 2);

    let back = program.sheet_module(2).unwrap();
    assert!(back.name.len() <= 15 && !back.name.is_empty());
    assert_eq!(back.blocks.len(), 6);
    assert!(back.locals.is_empty());
    assert_eq!(back.tags.len(), 2);
    assert!(back.tags[0].reads && back.tags[0].writes);
    assert_eq!(back.tags[0].init, Some(ParamValue::Float(0.0)));
    assert!(!back.tags[1].reads && back.tags[1].writes);

    let main = program.sheet_module(1).unwrap();
    assert_eq!(main.name, "sheet1");
    assert_eq!(main.blocks.len(), 4);
    assert_eq!(main.tags.len(), 2);
    assert!(main
        .tags
        .iter()
        .all(|t| t.reads && !t.writes && t.init.is_none()));

    // No block belongs to both sheets.
    assert!(main.blocks.iter().all(|b| !back.blocks.contains(b)));
}

#[test]
fn variable_on_one_sheet_stays_local() {
    let lib = library();
    let mut project = background();
    project.logic.vars[0].retain = false;
    // The readers move to the sheet of the writer: the variable no longer crosses the border.
    for id in ["shown", "reached", "led"] {
        project.layout.blocks.get_mut(id).unwrap().sheet = 2;
    }
    let program = Program::build(&project, &lib).unwrap();
    let table = program.tag_table();
    assert_eq!(table.len(), 1);
    assert_eq!(table[0].kind, TagKind::Signal("press.out".into()));
    let back = program.sheet_module(2).unwrap();
    assert_eq!(back.locals.len(), 1);
    assert_eq!(back.locals[0].name, "Нажатий всего");
}

#[test]
fn retained_number_is_kept_in_the_module() {
    // Numbers and flags are kept in flash by the master; only texts are refused.
    let lib = library();
    let project = background();
    let program = Program::build(&project, &lib).unwrap();
    let module = program.sheet_module(2).unwrap();
    // The variable crosses the sheets, so the owner (this sheet) restores and saves it.
    assert!(module.tags.iter().any(|t| {
        t.init.is_some() && t.tag.kind == TagKind::Var("Нажатий всего".into())
    }));
    assert!(program
        .wasm_module(2)
        .unwrap()
        .manifest
        .contains("\"retain\""));
}

#[test]
fn empty_sheet_is_refused() {
    let lib = library();
    let project = background();
    let program = Program::build(&project, &lib).unwrap();
    assert_eq!(program.sheet_module(7).unwrap_err(), SheetError::Empty(7));
}

#[test]
fn pins_become_imports_and_rights() {
    let lib = library();
    let mut project = background();
    project.logic.vars[0].retain = false;
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module(1).unwrap();
    assert!(
        built.manifest.contains("allow = [\"gpio_write\"]"),
        "{}",
        built.manifest
    );
}

#[test]
fn unsupported_block_is_refused_with_its_name() {
    let lib = library();
    let mut project = background();
    project.logic.vars[0].retain = false;
    // One chip used from two sheets: its word cannot be shared between modules.
    for id in ["blink_led", "button"] {
        project
            .logic
            .blocks
            .iter_mut()
            .find(|b| b.id == id)
            .unwrap()
            .expander = Some("pcf".into());
    }
    project.logic.expanders.push(crab_core::expander::Expander {
        id: "pcf".into(),
        chip: crab_core::ExpanderChip::Pcf8575,
        address: 0x20,
        sda: -1,
        scl: -1,
        period: "20ms".into(),
    });
    let program = Program::build(&project, &lib).unwrap();
    let err = program.wasm_module(1).unwrap_err().to_string();
    assert!(
        err.contains("blink_led") && err.contains("не поддержан"),
        "{err}"
    );
}

/// The `crab.mem` section of a module: bytes of memory it needs (`specs/mcu-memory`).
fn mem_section(wasm: &[u8]) -> u32 {
    let name = crab_codegen::wasm::MEM_SECTION.as_bytes();
    let at = wasm
        .windows(name.len())
        .position(|w| w == name)
        .expect("no crab.mem section")
        + name.len();
    u32::from_le_bytes(wasm[at..at + 4].try_into().unwrap())
}

#[test]
fn module_tells_the_memory_it_needs() {
    let lib = library();
    let mut project = background();
    project.logic.vars[0].retain = false;
    let program = Program::build(&project, &lib).unwrap();
    // No strings: no memory at all.
    assert_eq!(mem_section(&program.wasm_module(1).unwrap().wasm), 0);

    // MQTT topics are text: the module needs a few hundred bytes, far below the 64 KB page.
    let project = Project::load(repo_path("examples/mqtt_wasm")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let mem = mem_section(&program.wasm_module(1).unwrap().wasm);
    assert!(mem > 64 && mem < 4096, "{mem}");
    assert_eq!(mem % 16, 0);
}

/// ABI 4: `step` and the back edge of every loop spend fuel and call `crab.yield` when it runs
/// out. The module of `examples/text_wasm` (reading a UART line loops) is kept as a WAT snapshot;
/// to accept changes run `UPDATE_SNAPSHOTS=1 cargo test` and review the diff in git.
#[test]
fn module_with_a_loop_spends_fuel() {
    let lib = library();
    let project = Project::load(repo_path("examples/text_wasm")).unwrap();
    let program = Program::build(&project, &lib).unwrap();
    let built = program.wasm_module(1).unwrap();
    assert!(built.manifest.contains("abi = 4"), "{}", built.manifest);
    let text = wasmprinter::print_bytes(&built.wasm).unwrap();
    assert!(text.contains("(import \"crab\" \"yield\" (func"), "{text}");
    let after_loop = text.split_once("loop").expect("a loop").1;
    let yield_in_loop = after_loop.find("call 6").expect("yield in the loop");
    assert!(yield_in_loop < after_loop.find("br_if").unwrap_or(usize::MAX));

    let path = repo_path("crates/crab-codegen/tests/snapshots/text_wasm.module.wat");
    let expected = std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.replace("\r\n", "\n"));
    if expected.as_deref() == Some(text.as_str()) {
        return;
    }
    if expected.is_none() || std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &text).unwrap();
        eprintln!("снимок записан: {}", path.display());
        return;
    }
    panic!("модуль отличается от снимка {}", path.display());
}
