//! `crablogic block check`: one user block folder (`block.toml`, `block.wasm`, `docs.md`) checked
//! on its own, every problem named with a hint. The block is also tried in a one-block project, so
//! what the sheet module generator would refuse shows up here, not on the board.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crab_core::library::{
    BlockDef, LibraryBlock, ParamKind, SignalType, BUS_UART_PORT, DOCS_FILE, ESP32_CPP_TEMPLATE,
    WASM_FILE,
};
use crab_core::{Error, Library, Project, Severity};

use crate::{GenError, Program};

/// One finding about a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDiag {
    pub severity: Severity,
    /// Short stable class: `toml`, `def`, `builtin`, `cpp`, `no-wasm`, `wasm`, `string`, `schema`, `gen`,
    /// `docs`, `sim`, `description`.
    pub code: &'static str,
    pub message: String,
    /// One Russian sentence: how to fix it.
    pub hint: &'static str,
}

/// Result of [`check_block`].
#[derive(Debug, Clone, Default)]
pub struct BlockCheck {
    /// Block type, when `block.toml` could be read.
    pub kind: Option<String>,
    pub diags: Vec<BlockDiag>,
    /// `block.wasm` imports `bus.*` functions.
    pub driver: bool,
    pub wasm_bytes: Option<usize>,
    /// Size of the sheet module generated for a one-block project.
    pub module_bytes: Option<usize>,
}

impl BlockCheck {
    pub fn has_errors(&self) -> bool {
        self.diags.iter().any(|d| d.severity == Severity::Error)
    }

    fn push(
        &mut self,
        severity: Severity,
        code: &'static str,
        message: String,
        hint: &'static str,
    ) {
        self.diags.push(BlockDiag {
            severity,
            code,
            message,
            hint,
        });
    }

    fn error(&mut self, code: &'static str, message: String, hint: &'static str) {
        self.push(Severity::Error, code, message, hint);
    }

    fn warning(&mut self, code: &'static str, message: String, hint: &'static str) {
        self.push(Severity::Warning, code, message, hint);
    }
}

const CPP_FILES: [&str; 2] = [ESP32_CPP_TEMPLATE, crab_core::native::NATIVE_FILE];

/// Checks the block folder `dir`; `lib` is the library the block would live in.
pub fn check_block(dir: &Path, lib: &Library) -> BlockCheck {
    let mut out = BlockCheck::default();
    let cpp: Vec<&str> = CPP_FILES
        .into_iter()
        .filter(|f| dir.join(f).is_file())
        .collect();
    let block = match LibraryBlock::load(dir) {
        Ok(b) => b,
        Err(e) => {
            // A broken native part also fails the load: the C++ file is the real news then.
            if cpp.is_empty() {
                let (code, hint) = match e {
                    Error::InvalidBlockDef { .. } => (
                        "def",
                        "Исправьте описание порта или параметра в block.toml по тексту ошибки.",
                    ),
                    _ => (
                        "toml",
                        "Исправьте block.toml: проверьте строку и столбец из ошибки.",
                    ),
                };
                out.error(code, e.to_string(), hint);
            }
            report_cpp(&mut out, &cpp);
            return out;
        }
    };
    out.kind = Some(block.def.info.kind.clone());
    let kind = block.def.info.kind.as_str();
    let same_dir = |other: &Path| {
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
        canon(other) == canon(dir)
    };
    if lib.is_builtin(kind) && lib.get(kind).is_some_and(|b| !same_dir(&b.dir)) {
        out.error(
            "builtin",
            format!("тип «{kind}» занят встроенным блоком"),
            "Выберите другое имя: поменяйте type в block.toml и имя папки.",
        );
    }
    for p in &block.def.params {
        let n = match &p.default {
            Some(crab_core::ParamValue::Int(n)) => *n as f64,
            Some(crab_core::ParamValue::Float(x)) => *x,
            _ => continue,
        };
        if p.min.is_some_and(|min| n < min) || p.max.is_some_and(|max| n > max) {
            out.error(
                "def",
                format!(
                    "значение по умолчанию {n} параметра «{}» вне его границ min/max",
                    p.name
                ),
                "Поправьте default, min или max параметра в block.toml.",
            );
        }
    }
    report_cpp(&mut out, &cpp);

    let bytes = std::fs::read(dir.join(WASM_FILE)).ok();
    match &bytes {
        None => out.error(
            "no-wasm",
            format!("в папке нет файла {WASM_FILE}"),
            "Соберите блок в WASM и положите block.wasm рядом с block.toml.",
        ),
        Some(bytes) => {
            out.wasm_bytes = Some(bytes.len());
            match crate::wasm::inspect_block_wasm(bytes) {
                Ok(info) => out.driver = info.driver,
                Err(e) => out.error(
                    "wasm",
                    e,
                    "Нужен экспорт step(i32) и импорты только crab.input/param/output/log/config и bus.*.",
                ),
            }
        }
    }

    let def = &block.def;
    // The display drivers take their model from `crab.config`; the generator leaves the cell
    // of their text parameter at 0 (see `wasm::user_block`).
    let text_params_ok = crate::wasm::takes_text_params(&block.def.info.kind);
    let has_string = def.inputs.iter().any(|p| p.ty == SignalType::String)
        || def.outputs.iter().any(|p| p.ty == SignalType::String)
        || (!text_params_ok && def.params.iter().any(|p| p.kind == ParamKind::Text));
    if has_string {
        out.error(
            "string",
            "строковые порты и параметры не передаются в block.wasm: он получает только числа"
                .to_owned(),
            "Уберите порты и параметры типа string или замените их числовыми.",
        );
    }

    if bytes.is_some() && !out.has_errors() {
        generate(&mut out, &block, lib);
    }

    if !dir.join(DOCS_FILE).is_file() {
        out.warning(
            "docs",
            format!("нет файла {DOCS_FILE}"),
            "Добавьте docs.md с описанием блока: оно показывается в справке.",
        );
    }
    if out.driver {
        for port in def.outputs.iter().filter(|p| p.sim.is_none()) {
            out.warning(
                "sim",
                format!("у выхода «{}» драйвера нет значения sim", port.name),
                "Добавьте sim = <значение> к выходу: с ним схема работает в симуляции на ПК.",
            );
        }
    }
    if def.info.description.trim().is_empty() {
        out.warning(
            "description",
            "пустое описание блока".to_owned(),
            "Заполните description в [block] одной фразой.",
        );
    }
    out
}

fn report_cpp(out: &mut BlockCheck, cpp: &[&str]) {
    for file in cpp {
        out.error(
            "cpp",
            format!("{file}: {}", crate::CPP_BLOCK_HINT),
            "Перепишите блок как block.wasm и удалите файл C++.",
        );
    }
}

/// Builds the sheet module of a one-block project with the checked block.
fn generate(out: &mut BlockCheck, block: &LibraryBlock, lib: &Library) {
    let mut lib = lib.clone();
    lib.insert(block.clone());
    let root = unique_temp();
    let result: Result<usize, (&'static str, String)> = (|| {
        let dir = one_block_project(&root, &block.def).map_err(|e| ("gen", e.to_string()))?;
        let project = Project::load(&dir).map_err(|e| ("gen", e.to_string()))?;
        let project = crate::prepare(&project, &lib);
        let program = Program::build(&project, &lib).map_err(|e| match e {
            GenError::Schema(diags) => {
                let msgs: Vec<String> = diags
                    .iter()
                    .filter(|d| d.severity == Severity::Error)
                    .take(3)
                    .map(|d| d.message.clone())
                    .collect();
                ("schema", msgs.join("; "))
            }
            e => ("gen", e.to_string()),
        })?;
        let mut size = 0;
        for sheet in program.sheets() {
            let module = program
                .wasm_module(sheet)
                .map_err(|e| ("gen", e.to_string()))?;
            size += module.wasm.len();
        }
        Ok(size)
    })();
    let _ = std::fs::remove_dir_all(&root);
    match result {
        Ok(size) => out.module_bytes = Some(size),
        Err(("schema", m)) => out.error(
            "schema",
            m,
            "Блок в одиночном проекте не проходит проверку схемы: поправьте порты и параметры.",
        ),
        Err((_, m)) => out.error(
            "gen",
            m,
            "Генератор модуля отказал: поправьте block.wasm или block.toml по тексту ошибки.",
        ),
    }
}

fn unique_temp() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!(
        "crab_block_check_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// GPIOs for trial pins: not the strapping or flash ones, and not the pins of the two UARTs of the
/// trial project (25, 27, 16, 17).
const PIN_POOL: [u32; 12] = [4, 5, 13, 14, 18, 19, 21, 22, 23, 26, 32, 33];

/// A project folder with only `def`'s block in it, under `root`: parameters without a default get
/// a trial value, every input a constant of its type, so the block itself is what is checked. Every
/// trial pin is a distinct GPIO (pins of one `bus` name share one).
pub fn one_block_project(root: &Path, def: &BlockDef) -> std::io::Result<PathBuf> {
    let kind = def.info.kind.as_str();
    let mut next_pin = 0usize;
    let mut take_pin = || {
        let pin = PIN_POOL[next_pin % PIN_POOL.len()];
        next_pin += 1;
        pin
    };
    let mut bus_pins: std::collections::BTreeMap<&str, u32> = std::collections::BTreeMap::new();
    let mut logic = format!("[[block]]\nid = \"b\"\ntype = \"{kind}\"\n");
    for p in def.params.iter().filter(|p| p.default.is_none()) {
        let value = match p.kind {
            ParamKind::Bool => "false".to_owned(),
            ParamKind::Int | ParamKind::Float => "1".to_owned(),
            // Points of a table read well as any other text.
            ParamKind::Text => "\"0:0, 10:10\"".to_owned(),
            ParamKind::Duration => "\"1s\"".to_owned(),
            ParamKind::Pin if p.bus.is_empty() => take_pin().to_string(),
            ParamKind::Pin => match bus_pins.get(p.bus.as_str()) {
                Some(pin) => pin.to_string(),
                None => {
                    let pin = take_pin();
                    bus_pins.insert(p.bus.as_str(), pin);
                    pin.to_string()
                }
            },
        };
        logic.push_str(&format!("{} = {value}\n", p.name));
    }
    // Web widgets live on a page of a network block.
    if kind.starts_with("web_") {
        logic.push_str("\n[[block]]\nid = \"net\"\ntype = \"wifi\"\nssid = \"a\"\n");
        if kind != "web_ui" {
            logic.push_str("\n[[block]]\nid = \"page\"\ntype = \"web_ui\"\n");
        }
    }
    let defaults = std::collections::BTreeMap::new();
    for (i, input) in def.inputs.iter().enumerate() {
        if !def
            .enabled(input.when.as_deref(), &defaults)
            .unwrap_or(true)
        {
            continue;
        }
        let source = match input.ty {
            SignalType::String => "type = \"text_const\"\nvalue = \"a\"".to_owned(),
            SignalType::Bool => format!("type = \"digital_in\"\npin = {}", take_pin()),
            SignalType::Int => {
                logic.push_str(&format!(
                    "\n[[block]]\nid = \"f{i}\"\ntype = \"constant\"\nvalue = 1\n\n\
                     [[link]]\nfrom = \"f{i}.out\"\nto = \"c{i}.in\"\n"
                ));
                "type = \"float_to_int\"".to_owned()
            }
            _ => "type = \"constant\"\nvalue = 1".to_owned(),
        };
        logic.push_str(&format!(
            "\n[[block]]\nid = \"c{i}\"\n{source}\n\n[[link]]\nfrom = \"c{i}.out\"\nto = \"b.{}\"\n",
            input.name
        ));
    }
    let dir = root.join(kind);
    std::fs::create_dir_all(&dir)?;
    let mut project =
        "[project]\nname = \"audit\"\n\n[board]\nfqbn = \"esp32:esp32:esp32\"\n".to_owned();
    // Blocks on a UART want it in the board settings; `uart_config` sets it up itself.
    let wants_uart = def.params.iter().any(|p| p.bus == BUS_UART_PORT) || kind != "uart_config";
    if wants_uart {
        project.push_str(concat!(
            "\n[[uart]]\nnumber = 1\nspeed = 9600\nformat = \"8N1\"\nrx = 25\ntx = 27\n\n",
            "[[uart]]\nnumber = 2\nspeed = 9600\nformat = \"8N1\"\nrx = 16\ntx = 17\n",
        ));
    }
    std::fs::write(dir.join("project.toml"), project)?;
    std::fs::write(dir.join("logic.toml"), logic)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(path: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    fn lib() -> Library {
        let (lib, errors) = Library::load(repo("library")).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        lib
    }

    const HEADER: &str = "[block]\ntype = \"twice\"\nname = \"Twice\"\nsymbol = \"x2\"\ngroup = \"Тест\"\ndescription = \"Удваивает вход.\"\n";
    const PORTS: &str = "\n[[input]]\nname = \"a\"\ntype = \"float\"\n\n[[output]]\nname = \"out\"\ntype = \"float\"\n";
    const TWICE_WAT: &str = r#"(module
        (import "crab" "input" (func $in (param i32) (result f64)))
        (import "crab" "output" (func $out (param i32 f64)))
        (func (export "step") (param i32)
          (call $out (i32.const 0) (f64.mul (call $in (i32.const 0)) (f64.const 2)))))"#;

    fn good_toml() -> String {
        format!("{HEADER}{PORTS}")
    }

    /// A block folder `twice` in a fresh temp dir; returns (root, folder).
    fn block_dir(toml: &str, wat: Option<&str>, docs: bool) -> (PathBuf, PathBuf) {
        let root = unique_temp();
        let dir = root.join("twice");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("block.toml"), toml).unwrap();
        if let Some(wat) = wat {
            std::fs::write(dir.join("block.wasm"), wat::parse_str(wat).unwrap()).unwrap();
        }
        if docs {
            std::fs::write(dir.join("docs.md"), "# Twice\n").unwrap();
        }
        (root, dir)
    }

    fn codes(check: &BlockCheck) -> Vec<&'static str> {
        check.diags.iter().map(|d| d.code).collect()
    }

    fn expect_error(toml: &str, wat: Option<&str>, code: &str) {
        let (root, dir) = block_dir(toml, wat, true);
        let check = check_block(&dir, &lib());
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            check
                .diags
                .iter()
                .any(|d| d.code == code && d.severity == Severity::Error),
            "want {code}: {:?}",
            check.diags
        );
    }

    #[test]
    fn a_good_logic_block_passes() {
        let (root, dir) = block_dir(&good_toml(), Some(TWICE_WAT), true);
        let check = check_block(&dir, &lib());
        let _ = std::fs::remove_dir_all(&root);
        assert!(check.diags.is_empty(), "{:?}", check.diags);
        assert!(check.module_bytes.is_some());
        assert!(!check.driver);
        assert_eq!(check.kind.as_deref(), Some("twice"));
    }

    #[test]
    fn a_library_driver_passes() {
        let check = check_block(&repo("library/dht22"), &lib());
        assert!(!check.has_errors(), "{:?}", check.diags);
        assert!(check.driver);
        assert!(check.module_bytes.is_some());
    }

    #[test]
    fn warnings_do_not_fail() {
        let toml = good_toml().replace("description = \"Удваивает вход.\"", "description = \"\"");
        let (root, dir) = block_dir(&toml, Some(TWICE_WAT), false);
        let check = check_block(&dir, &lib());
        let _ = std::fs::remove_dir_all(&root);
        assert!(!check.has_errors(), "{:?}", check.diags);
        assert_eq!(codes(&check), ["docs", "description"]);
    }

    #[test]
    fn a_bad_port_name_is_a_definition_error() {
        let toml = good_toml().replace("name = \"a\"", "name = \"a b\"");
        expect_error(&toml, Some(TWICE_WAT), "def");
        let dup = good_toml().replace("name = \"out\"", "name = \"a\"");
        expect_error(&dup, Some(TWICE_WAT), "def");
    }

    #[test]
    fn a_bad_signal_type_is_a_toml_error() {
        let toml = good_toml().replace("\"float\"\n\n[[output", "\"double\"\n\n[[output");
        expect_error(&toml, Some(TWICE_WAT), "toml");
    }

    #[test]
    fn a_bad_param_is_a_definition_error() {
        let out_of_range = format!(
            "{}\n[[param]]\nname = \"k\"\ntype = \"int\"\ndefault = 50\nmin = 0\nmax = 10\n",
            good_toml()
        );
        expect_error(&out_of_range, Some(TWICE_WAT), "def");
        let text_default = format!(
            "{}\n[[param]]\nname = \"k\"\ntype = \"int\"\ndefault = \"abc\"\n",
            good_toml()
        );
        expect_error(&text_default, Some(TWICE_WAT), "def");
    }

    #[test]
    fn wasm_without_step_is_refused() {
        expect_error(
            &good_toml(),
            Some(r#"(module (func (export "init")))"#),
            "wasm",
        );
    }

    #[test]
    fn a_foreign_import_is_refused() {
        let wat = r#"(module
            (import "env" "foo" (func))
            (func (export "step") (param i32) (call 0)))"#;
        expect_error(&good_toml(), Some(wat), "wasm");
    }

    #[test]
    fn a_missing_wasm_is_named() {
        expect_error(&good_toml(), None, "no-wasm");
    }

    #[test]
    fn a_string_port_is_refused() {
        let toml = good_toml().replace("\"float\"\n\n[[output", "\"string\"\n\n[[output");
        expect_error(&toml, Some(TWICE_WAT), "string");
    }

    /// The reference blocks of the AI instruction (`skills/crablogic-block/examples`) must stay clean.
    #[test]
    fn skill_examples_pass() {
        let examples =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/crablogic-block/examples");
        let mut seen = 0;
        for example in std::fs::read_dir(&examples).unwrap().flatten() {
            let blocks = example.path().join("project/library");
            let Ok(entries) = std::fs::read_dir(&blocks) else {
                continue;
            };
            for block in entries.flatten() {
                let check = check_block(&block.path(), &lib());
                assert!(
                    check.diags.is_empty(),
                    "{}: {:?}",
                    block.path().display(),
                    check.diags
                );
                assert!(check.module_bytes.is_some());
                seen += 1;
            }
        }
        assert_eq!(seen, 3);
    }

    #[test]
    fn a_builtin_type_name_is_refused() {
        let (root, _) = block_dir(&good_toml(), None, true);
        let dir = root.join("mul");
        std::fs::create_dir_all(&dir).unwrap();
        let toml = good_toml().replace("type = \"twice\"", "type = \"mul\"");
        assert!(toml.contains("type = \"mul\""), "{toml}");
        std::fs::write(dir.join("block.toml"), toml).unwrap();
        std::fs::write(dir.join("block.wasm"), wat::parse_str(TWICE_WAT).unwrap()).unwrap();
        let check = check_block(&dir, &lib());
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            check.diags.iter().any(|d| d.code == "builtin"),
            "{:?}",
            check.diags
        );
    }

    #[test]
    fn a_cpp_template_is_refused() {
        let (root, dir) = block_dir(&good_toml(), Some(TWICE_WAT), true);
        std::fs::write(dir.join("esp32.cpp.tmpl"), "// {{ id }}\n").unwrap();
        let check = check_block(&dir, &lib());
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            check.diags.iter().any(|d| d.code == "cpp"),
            "{:?}",
            check.diags
        );
    }
}
