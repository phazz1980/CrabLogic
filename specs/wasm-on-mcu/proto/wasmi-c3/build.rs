//! Turns the prototype `.wat` modules of `specs/wasm-on-mcu/proto` into `.wasm` in OUT_DIR.
use std::{env, fs, path::PathBuf};

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let proto = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("..");
    for name in ["blink", "counter", "hang"] {
        let src = proto.join(format!("{name}.wat"));
        println!("cargo:rerun-if-changed={}", src.display());
        let text = fs::read_to_string(&src).unwrap();
        fs::write(out.join(format!("{name}.wasm")), wat::parse_str(&text).unwrap()).unwrap();
        if name == "counter" {
            // Same module with a 1-byte page size (custom-page-sizes proposal): memory of exactly
            // 4096 bytes instead of a 64 KiB page.
            let small = text.replace(
                "(memory (export \"memory\") 1)",
                "(memory (export \"memory\") 4096 (pagesize 1))",
            );
            assert_ne!(small, text);
            fs::write(out.join("counter_p1.wasm"), wat::parse_str(&small).unwrap()).unwrap();
        }
    }
    println!("cargo:rerun-if-changed=modules");
}
