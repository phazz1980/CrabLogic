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
