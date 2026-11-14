//! `crablogic report-decode <zip>`: the crash dump of a bug report as a readable stack
//! (`specs/bug-report`). The work is done by `esp_coredump` (Python, from the ESP-IDF environment)
//! with the gdb of the chip and the ELF of the master the board ran.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The chip a crash dump came from, by the machine field of its ELF header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip {
    Esp32,
    Esp32c3,
}

impl Chip {
    /// The dump as stored in flash: a header of ESP-IDF, then the ELF (or the ELF alone).
    pub fn of_elf(image: &[u8]) -> Option<Chip> {
        let head = &image[..image.len().min(64)];
        let at = head.windows(4).position(|w| w == b"\x7fELF")?;
        let elf = image.get(at..at + 20)?;
        match u16::from_le_bytes([elf[18], elf[19]]) {
            94 => Some(Chip::Esp32),
            243 => Some(Chip::Esp32c3),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Chip::Esp32 => "esp32",
            Chip::Esp32c3 => "esp32c3",
        }
    }

    fn target(self) -> &'static str {
        match self {
            Chip::Esp32 => "xtensa-esp32-espidf",
            Chip::Esp32c3 => "riscv32imc-esp-espidf",
        }
    }

    fn gdb(self) -> &'static str {
        match self {
            Chip::Esp32 => "xtensa-esp32-elf-gdb",
            Chip::Esp32c3 => "riscv32-esp-elf-gdb",
        }
    }
}

/// One entry of the report archive, if present.
fn entry(zip_path: &Path, name: &str) -> Result<Option<Vec<u8>>, String> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| format!("не удалось открыть {}: {e}", zip_path.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("{} — не ZIP-архив отчёта: {e}", zip_path.display()))?;
    let Ok(mut f) = zip.by_name(name) else {
        return Ok(None);
    };
    let mut data = Vec::new();
    f.read_to_end(&mut data)
        .map_err(|e| format!("ошибка чтения {name} из архива: {e}"))?;
    Ok(Some(data))
}

/// Folders to look in for the ESP-IDF tools: the program folder and the repository (developer).
fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        roots.extend(dir.ancestors().take(4).map(Path::to_path_buf));
    }
    if let Ok(dir) = std::env::current_dir() {
        roots.extend(dir.ancestors().take(4).map(Path::to_path_buf));
    }
    roots
}

/// The ELF of the master: given, from the program folder, or the last local build.
fn master_elf(chip: Chip, given: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = given {
        return Some(p.to_path_buf());
    }
    roots().into_iter().find_map(|root| {
        [
            root.join("firmware")
                .join(chip.name())
                .join("crab-master-rs.elf"),
            root.join(".m")
                .join(chip.target())
                .join("release")
                .join("crab-master-rs"),
        ]
        .into_iter()
        .find(|p| p.is_file())
    })
}

/// Python of the ESP-IDF environment (has `esp_coredump`), or `python` from PATH.
fn python() -> PathBuf {
    for root in roots() {
        let envs = root.join(".embuild").join("espressif").join("python_env");
        if let Ok(dirs) = std::fs::read_dir(&envs) {
            for d in dirs.flatten() {
                let exe = d.path().join("Scripts").join("python.exe");
                if exe.is_file() {
                    return exe;
                }
                let exe = d.path().join("bin").join("python");
                if exe.is_file() {
                    return exe;
                }
            }
        }
    }
    PathBuf::from("python")
}

/// gdb of the chip: from PATH or from the ESP-IDF tools folder.
fn gdb(chip: Chip) -> Option<PathBuf> {
    let exe = format!("{}{}", chip.gdb(), std::env::consts::EXE_SUFFIX);
    if let Some(path) = std::env::var_os("PATH") {
        if let Some(p) = std::env::split_paths(&path)
            .map(|d| d.join(&exe))
            .find(|p| p.is_file())
        {
            return Some(p);
        }
    }
    for root in roots() {
        let tools = root.join(".embuild").join("espressif").join("tools");
        if let Some(p) = find_file(&tools, &exe, 5) {
            return Some(p);
        }
    }
    None
}

fn find_file(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut dirs = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            dirs.push(p);
        } else if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
    }
    if depth == 0 {
        return None;
    }
    dirs.into_iter()
        .find_map(|d| find_file(&d, name, depth - 1))
}

/// Runs the decoding and prints its result.
pub fn run(zip_path: &Path, elf: Option<&Path>) -> Result<(), String> {
    if let Some(md) = entry(zip_path, "report.md")? {
        let text = String::from_utf8_lossy(&md);
        for line in text.lines().filter(|l| {
            let l = l.to_lowercase();
            l.contains("мастер") || l.contains("плата") || l.contains("перезапуск")
        }) {
            println!("{line}");
        }
    }
    let Some(core) = entry(zip_path, "coredump.elf")? else {
        return Err("в отчёте нет дампа сбоя платы (coredump.elf)".into());
    };
    let chip =
        Chip::of_elf(&core).ok_or("coredump.elf — не дамп ESP32 / ESP32-C3 (неизвестный ELF)")?;
    println!("Дамп: {} байт, чип {}", core.len(), chip.name());
    let elf = master_elf(chip, elf).ok_or_else(|| {
        format!(
            "не найден ELF мастера для {}: укажите --elf <файл> (firmware/{}/crab-master-rs.elf той же версии, что в отчёте)",
            chip.name(),
            chip.name()
        )
    })?;
    println!("ELF мастера: {}", elf.display());
    let Some(gdb) = gdb(chip) else {
        return Err(format!(
            "не найден {} — он нужен для расшифровки стека. Установите его из ESP-IDF:\n  python .embuild/espressif/esp-idf/<версия>/tools/idf_tools.py install {}-gdb\nили добавьте его папку в PATH",
            chip.gdb(),
            if chip == Chip::Esp32 { "xtensa-esp-elf" } else { "riscv32-esp-elf" }
        ));
    };
    let tmp = std::env::temp_dir().join(format!("crablogic-core-{}.elf", std::process::id()));
    std::fs::write(&tmp, &core)
        .map_err(|e| format!("не удалось записать {}: {e}", tmp.display()))?;
    let out = Command::new(python())
        .args(["-m", "esp_coredump", "--chip", chip.name(), "info_corefile"])
        .arg("--core")
        .arg(&tmp)
        .args(["--core-format", "raw", "--gdb"])
        .arg(&gdb)
        .arg(&elf)
        .output();
    let _ = std::fs::remove_file(&tmp);
    let out = out.map_err(|e| format!("не удалось запустить Python с esp_coredump: {e}"))?;
    print!("{}", String::from_utf8_lossy(&out.stdout));
    if !out.status.success() {
        return Err(format!(
            "esp_coredump завершился с ошибкой:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chip_from_the_elf_header() {
        let mut h = vec![0u8; 20];
        h[..4].copy_from_slice(b"\x7fELF");
        h[18] = 94;
        assert_eq!(Chip::of_elf(&h), Some(Chip::Esp32));
        h[18] = 243;
        assert_eq!(Chip::of_elf(&h), Some(Chip::Esp32c3));
        h[18] = 3;
        assert_eq!(Chip::of_elf(&h), None);
        assert_eq!(Chip::of_elf(b"PK\x03\x04"), None);
        // As read from the board: 20 bytes of the ESP-IDF header first.
        let mut stored = vec![0u8; 20];
        stored.extend_from_slice(&h);
        stored[20 + 18] = 243;
        assert_eq!(Chip::of_elf(&stored), Some(Chip::Esp32c3));
    }
}
