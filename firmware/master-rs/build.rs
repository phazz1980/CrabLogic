//! Board build only: ESP-IDF link arguments and the built-in test modules
//! (`modules/*.wat` → `.wasm` for the interpreter and `.aot` for XIP).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

/// ESP32-C3 is RV32IMC, soft float, no atomics; `--xip` makes the image runnable from flash.
const WAMRC_C3: &[&str] = &[
    "--target=riscv32",
    "--target-abi=ilp32",
    "--cpu=generic-rv32",
    "--cpu-features=+m,+c,-a,-f,-d",
    "--xip",
    "--disable-simd",
    "--disable-ref-types",
    "--bounds-checks=1",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("espidf") {
        return;
    }
    embuild::espidf::sysenv::output();

    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    device_page(&root, &out);
    println!("cargo:rerun-if-env-changed=CRAB_WAMRC");
    let wamrc = env::var("CRAB_WAMRC")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join(".deps/wamrc/wamrc.exe"));
    if !wamrc.exists() {
        panic!(
            "не найден wamrc ({}): положите его в .deps/wamrc или задайте CRAB_WAMRC (см. README)",
            wamrc.display()
        );
    }

    let modules = root.join("modules");
    println!("cargo:rerun-if-changed={}", modules.display());
    for entry in fs::read_dir(&modules).expect("modules/") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("wat") {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap();
        let wasm = wat::parse_file(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let wasm_path = out.join(format!("{name}.wasm"));
        fs::write(&wasm_path, wasm).unwrap();
        aot(&wamrc, &wasm_path, &out.join(format!("{name}.aot")));
    }
}

/// The web page of the device (`crab_blocks::device_page`): cut out of the raw string in the editor
/// sources, so the firmware serves the same page and does not depend on `crab-core`.
fn device_page(root: &Path, out: &Path) {
    let src = root.join("../../crates/crab-blocks/src/web.rs");
    println!("cargo:rerun-if-changed={}", src.display());
    let text = fs::read_to_string(&src).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
    const OPEN: &str = "R\"CRABUI(";
    const CLOSE: &str = ")CRABUI\"";
    let start = text
        .find(OPEN)
        .expect("page start in crab-blocks/src/web.rs")
        + OPEN.len();
    let len = text[start..]
        .find(CLOSE)
        .expect("page end in crab-blocks/src/web.rs");
    fs::write(out.join("device_page.html"), &text[start..start + len]).unwrap();
}

fn aot(wamrc: &Path, wasm: &Path, aot: &Path) {
    let run = Command::new(wamrc)
        .args(WAMRC_C3)
        .arg("-o")
        .arg(aot)
        .arg(wasm)
        .output()
        .unwrap_or_else(|e| panic!("wamrc не запущен: {e}"));
    if !run.status.success() {
        panic!(
            "wamrc: {}\n{}{}",
            wasm.display(),
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
    }
}
