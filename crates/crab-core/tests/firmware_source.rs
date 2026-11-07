use std::path::PathBuf;

use crab_core::library::{FirmwareSource, LibraryBlock, ESP32_CPP_TEMPLATE, WASM_FILE};
use crab_core::Library;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

#[test]
fn template_wins_over_module() {
    let dir = std::env::temp_dir().join(format!("crab-fwsrc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let block_dir = dir.join("sample_rs");
    std::fs::create_dir_all(&block_dir).unwrap();
    std::fs::write(
        block_dir.join("block.toml"),
        r#"[block]
type = "sample_rs"
name = "Sample"
group = "Test"
description = "A block with a module of its own."
memory = true

[[input]]
name = "in"
type = "bool"
description = "In"

[[output]]
name = "out"
type = "bool"
description = "Out"
"#,
    )
    .unwrap();

    let block = LibraryBlock::load(&block_dir).unwrap();
    assert_eq!(block.firmware_source(), FirmwareSource::None);

    std::fs::write(block_dir.join(WASM_FILE), b"\0asm\x01\0\0\0").unwrap();
    assert_eq!(block.firmware_source(), FirmwareSource::Module);

    std::fs::write(block_dir.join(ESP32_CPP_TEMPLATE), "// @loop\n").unwrap();
    assert_eq!(block.firmware_source(), FirmwareSource::Template);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn library_blocks_report_their_source() {
    let (lib, _) = Library::load(repo_path("library")).unwrap();
    assert_eq!(
        lib.get("ethernet").unwrap().firmware_source(),
        FirmwareSource::None
    );
}
