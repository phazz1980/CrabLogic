//! Disk log of the editor (`logs/editor.log`) and the crash note (`panic.txt`), both kept
//! for the bug report (`specs/bug-report`). Nothing here may fail the editor: IO errors
//! are ignored.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Size after which `editor.log` is moved to `editor.1.log`.
const LIMIT: u64 = 512 * 1024;
const CURRENT_NAME: &str = "editor.log";
const OLD_NAME: &str = "editor.1.log";

/// Open log file with its size, so a line costs one write and no metadata call.
struct Writer {
    dir: PathBuf,
    file: Option<File>,
    size: u64,
    limit: u64,
}

impl Writer {
    fn new(dir: PathBuf, limit: u64) -> Self {
        Self {
            dir,
            file: None,
            size: 0,
            limit,
        }
    }

    fn open(&mut self) -> Option<&mut File> {
        if self.file.is_none() {
            std::fs::create_dir_all(&self.dir).ok()?;
            let path = self.dir.join(CURRENT_NAME);
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()?;
            self.size = file.metadata().map(|m| m.len()).unwrap_or(0);
            self.file = Some(file);
        }
        self.file.as_mut()
    }

    fn append(&mut self, line: &str) {
        let add = line.len() as u64 + 1;
        if self.open().is_none() {
            return;
        }
        if self.size > 0 && self.size + add > self.limit {
            // Close before the rename: Windows will not move an open file.
            self.file = None;
            let _ = std::fs::rename(self.dir.join(CURRENT_NAME), self.dir.join(OLD_NAME));
            self.size = 0;
            if self.open().is_none() {
                return;
            }
        }
        if let Some(file) = self.file.as_mut() {
            if writeln!(file, "{line}").is_ok() {
                self.size += add;
            }
        }
    }
}

static WRITER: Mutex<Option<Writer>> = Mutex::new(None);

/// Folder of the logs: `<config>/logs`.
fn dir() -> Option<PathBuf> {
    crab_core::local::config_dir().map(|d| d.join("logs"))
}

/// Appends `<local time> <level> <text>` to the disk log.
pub fn write(level: &str, text: &str) {
    let Ok(mut guard) = WRITER.lock() else { return };
    if guard.is_none() {
        let Some(dir) = dir() else { return };
        *guard = Some(Writer::new(dir, LIMIT));
    }
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let text = text.replace('\n', " | ");
    if let Some(w) = guard.as_mut() {
        w.append(&format!("{stamp} {level} {text}"));
    }
}

/// Both log files, the older first.
pub fn read_all() -> Vec<u8> {
    match dir() {
        Some(dir) => read_dir(&dir),
        None => Vec::new(),
    }
}

fn read_dir(dir: &Path) -> Vec<u8> {
    let mut out = std::fs::read(dir.join(OLD_NAME)).unwrap_or_default();
    out.extend(std::fs::read(dir.join(CURRENT_NAME)).unwrap_or_default());
    out
}

fn panic_path() -> Option<PathBuf> {
    crab_core::local::config_dir().map(|d| d.join("panic.txt"))
}

/// Installs the panic hook: the note goes to `panic.txt` and the disk log, then the
/// previous hook runs.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // The hook must never panic itself.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| note_panic(info)));
        previous(info);
    }));
}

fn note_panic(info: &std::panic::PanicHookInfo<'_>) {
    let thread = std::thread::current();
    let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "(no message)".to_owned()
    };
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "(unknown)".to_owned());
    let text = format!(
        "time: {}\nversion: {}\nthread: {}\nmessage: {}\nlocation: {}\n\n{}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        crate::update::CURRENT,
        thread.name().unwrap_or("(unnamed)"),
        message,
        location,
        std::backtrace::Backtrace::force_capture()
    );
    if let Some(path) = panic_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, &text);
    }
    write("PANIC", &format!("{message} at {location}"));
}

/// Text of `panic.txt` left by a crash of the previous run.
pub fn pending_panic() -> Option<String> {
    let text = std::fs::read_to_string(panic_path()?).ok()?;
    Some(text)
}

/// Marks the crash as handled: `panic.txt` becomes `panic.old.txt`.
pub fn archive_panic() {
    if let Some(path) = panic_path() {
        let _ = std::fs::rename(&path, path.with_file_name("panic.old.txt"));
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_file_becomes_the_old_one() {
        let dir = std::env::temp_dir().join(format!("crab-editlog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut w = Writer::new(dir.clone(), 100);
        for i in 0..30 {
            w.append(&format!("line {i:02} 0123456789"));
        }
        let now = std::fs::metadata(dir.join(CURRENT_NAME)).unwrap().len();
        let old = std::fs::metadata(dir.join(OLD_NAME)).unwrap().len();
        assert!(now <= 100 && old <= 100 && old > 0, "{now} {old}");
        let all = String::from_utf8(read_dir(&dir)).unwrap();
        assert!(all.ends_with("line 29 0123456789\n"));
        assert!(all.find("line 28").unwrap() < all.find("line 29").unwrap());
        assert!(!all.contains("line 00"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
