use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crab_core::{Library, Project};

use super::common::find_library;

/// `board check`: one board folder with every problem named (a broken board is otherwise only
/// skipped by the library, and its blocks show up as unknown types).
pub(crate) fn run_board_check(dir: &Path, library: Option<&Path>) -> ExitCode {
    use crab_core::boards::{Boards, UserBoard};
    let board = match UserBoard::load(dir) {
        Ok(board) => board,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut bad = 0;
    let info = &board.def.info;
    if crab_build::flash::MasterChip::of_fqbn(&info.fqbn).is_none() {
        eprintln!(
            "ошибка: мастер на Rust есть только для ESP32 и ESP32-C3, а FQBN «{}» — другой чип",
            info.fqbn
        );
        bad += 1;
    }
    if !info.options.is_empty() {
        eprintln!(
            "предупреждение: [board.options] ({}) были нужны только сборке скетча (её больше нет) — мастер на Rust их не читает, уберите",
            info.options.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    // Device blocks are made from built-in blocks: an unknown `kind` or a bad parameter shows here.
    let here = std::env::current_dir().unwrap_or_default();
    match library
        .map(Path::to_path_buf)
        .or_else(|| find_library(&here))
        .map(Library::load)
    {
        Some(Ok((mut lib, _))) => {
            let mut boards = Boards::default();
            boards.insert(board.clone());
            for e in lib.set_boards(boards) {
                eprintln!("ошибка: {e}");
                bad += 1;
            }
            for dev in &board.def.devices {
                let kind = format!("{}_{}", info.id, dev.id);
                if lib.get(&kind).is_some() {
                    println!(
                        "блок периферии: {kind} («{}: {}», каналов {})",
                        info.name,
                        dev.name,
                        dev.channels.len()
                    );
                }
            }
        }
        _ => {
            eprintln!(
                "предупреждение: библиотека блоков не найдена (--library): периферия не проверена"
            );
        }
    }
    println!(
        "Плата «{}» ({}, {}): выводов {}, выводов расширителей {}, устройств {}",
        info.name,
        info.id,
        info.chip,
        board.def.pins.len(),
        board.def.expander_pins.len(),
        board.def.devices.len()
    );
    if !board.dir.join("docs.md").is_file() {
        println!("предупреждение: нет docs.md с описанием платы");
    }
    if bad == 0 {
        println!("Ошибок нет");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// `flash-master`: the prebuilt Rust master for the board of the project (or `--chip`).
pub(crate) fn run_flash_master(
    project: Option<&Path>,
    chip: Option<&str>,
    port: Option<String>,
    firmware: Option<PathBuf>,
) -> Result<(), String> {
    use crab_build::flash;
    let fqbn = match (project, chip) {
        (_, Some(chip)) => format!("esp32:esp32:{chip}"),
        (Some(project), None) => Project::load(project)
            .map_err(|e| e.to_string())?
            .config
            .board
            .fqbn
            .clone(),
        (None, None) => return Err("укажите проект или --chip esp32 / esp32c3".into()),
    };
    let dir = firmware
        .or_else(flash::firmware_dir)
        .ok_or("не найдена папка firmware с образами мастера (укажите --firmware)")?;
    let image = flash::find_image(&dir, &fqbn).map_err(|e| e.to_string())?;
    let port = match port {
        Some(port) => port,
        None => flash::single_usb_port().map_err(|e| e.to_string())?,
    };
    let started = std::time::Instant::now();
    flash::flash_master(&image, &port, &mut |line| println!("{line}"))
        .map_err(|e| e.to_string())?;
    println!(
        "Мастер-прошивка записана на {port} за {:.1} с",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}
