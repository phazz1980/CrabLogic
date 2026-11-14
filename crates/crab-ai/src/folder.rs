//! [`Context`] over a project file on disk: used by the MCP server and usable
//! without the editor. The project is re-read before each command so changes
//! saved by the editor are seen, and saved after each edit.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crab_codegen::Program;
use crab_core::{Library, LocalValues, Project};

use crate::Context;

pub struct FolderContext {
    lib_dir: PathBuf,
    project: Project,
    lib: Library,
}

impl FolderContext {
    /// Opens a project file, or a folder holding one (see [`Project::load`]).
    /// User blocks the project brings and the library lacks are loaded from a session
    /// folder; the user library is not touched.
    pub fn open(path: &Path, lib_dir: &Path) -> Result<Self, String> {
        let mut ctx = Self {
            lib_dir: lib_dir.to_owned(),
            project: Project::load(path).map_err(|e| e.to_string())?,
            lib: Library::default(),
        };
        ctx.load_library().map_err(|e| e.to_string())?;
        Ok(ctx)
    }

    /// Session folder of the project's own blocks for this process.
    fn session() -> std::path::PathBuf {
        std::env::temp_dir()
            .join("crablogic_mcp_blocks")
            .join(std::process::id().to_string())
    }

    fn load_library(&mut self) -> crab_core::Result<Vec<crab_core::Error>> {
        let (mut lib, mut errors) = Library::load_with_user(&self.lib_dir)?;
        let (_, more) =
            crab_core::packed::add_project_blocks(&self.project.path, &Self::session(), &mut lib);
        errors.extend(more);
        self.lib = lib;
        Ok(errors)
    }

    /// Re-reads the project from disk (call before each command).
    pub fn refresh(&mut self) -> Result<(), String> {
        // A converted old-format folder has no file until the first edit.
        if self.project.path.is_file() {
            self.project = Project::load(&self.project.path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

impl Context for FolderContext {
    fn project(&self) -> &Project {
        &self.project
    }

    fn library(&self) -> &Library {
        &self.lib
    }

    fn library_dir(&self) -> PathBuf {
        crab_core::user_library_dir(&self.lib_dir)
    }

    fn edit(
        &mut self,
        edit: &mut dyn FnMut(&mut Project) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut changed = self.project.clone();
        edit(&mut changed)?;
        // A password set by AI goes to the program, not into the project file.
        let mut store = LocalValues::open_default().map_err(|e| e.to_string())?;
        if changed.take_local(&self.lib, &mut store) {
            store.save().map_err(|e| e.to_string())?;
        }
        if let Some(warning) =
            crab_pdf::save_project(&changed, &self.lib).map_err(|e| e.to_string())?
        {
            eprintln!("{warning}");
        }
        self.project = changed;
        Ok(())
    }

    fn upload_modules(&mut self, port: Option<String>, debug: bool) -> Result<String, String> {
        upload_with_cli(&self.project, port, debug)
    }

    fn live_values(&mut self) -> Result<Vec<(String, String, bool)>, String> {
        let board = &self.project.config.board;
        let link = crab_build::project_link(board)?;
        let project = self.project.without_disabled();
        let program = Program::build(&project, &self.lib).map_err(|e| e.to_string())?;
        let table = crab_codegen::debug::debug_table(&program);
        let modules = crab_build::debug_modules(&program, &table, &project.work_dir())?;
        let connector = match link {
            crab_build::Chosen::Serial(port) => crab_debug::transport::serial(&port),
            crab_build::Chosen::Net(host) => {
                let password = &self.project.config.project.password;
                if password.is_empty() {
                    return Err(
                        "плата задана адресом по сети, но у проекта нет пароля ([project] password)"
                            .into(),
                    );
                }
                crab_debug::transport::tcp(&host, password)
            }
        };
        let session = crab_debug::MasterSession::start(table.clone(), modules, connector, || {});
        // Wait for a full set of values or give up after a few seconds.
        let mut live = session.snapshot();
        for _ in 0..50 {
            thread::sleep(Duration::from_millis(100));
            live = session.snapshot();
            if live.schema_matches == Some(false)
                || (live.schema_matches == Some(true) && live.values.iter().all(Option::is_some))
            {
                break;
            }
        }
        session.stop();
        if live.schema_matches != Some(true) {
            return Err(format!("нет живых значений: {}", live.status));
        }
        Ok(table
            .entries
            .iter()
            .zip(live.values.iter().zip(&live.forced))
            .map(|(e, (v, f))| {
                let name = match &e.source {
                    crab_debug::Source::Output(p) => p.to_string(),
                    crab_debug::Source::Var(v) => format!("var {v}"),
                };
                (
                    name,
                    v.map_or("—".to_owned(), |v| v.to_string()),
                    f.is_some(),
                )
            })
            .collect())
    }

    fn reload_library(&mut self) -> Result<(), String> {
        let errors = self.load_library().map_err(|e| e.to_string())?;
        match errors.first() {
            Some(e) => Err(e.to_string()),
            None => Ok(()),
        }
    }
}

/// The board address when `port` is a network one (`tcp:` prefix, or a dot as in `192.168.1.5`),
/// `None` for a serial port like `COM3`.
fn network_address(port: &str) -> Option<&str> {
    let port = port.trim();
    if let Some(host) = port.strip_prefix("tcp:") {
        return Some(host.trim());
    }
    (port.contains('.') && !port.contains(['/', '\\'])).then_some(port)
}

/// Uploads the modules of `project` through the command line of the program (`crablogic gen-wasm
/// --upload`, found next to this executable): it owns the whole upload (planning, AOT, replacing
/// the modules in place). Returns the tail of its output.
fn upload_with_cli(project: &Project, port: Option<String>, debug: bool) -> Result<String, String> {
    let exe_name = if cfg!(windows) {
        "crablogic.exe"
    } else {
        "crablogic"
    };
    let cli = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(exe_name)))
        .filter(|p| p.is_file())
        .ok_or(
            "рядом с программой нет crablogic: залейте модули командой crablogic gen-wasm --upload",
        )?;
    let mut command = std::process::Command::new(cli);
    command.arg("gen-wasm").arg(&project.path).arg("--upload");
    if debug {
        command.arg("--watch");
    }
    // Without an explicit port the command line chooses by the project: USB, else the network.
    if let Some(port) = port {
        match network_address(&port) {
            Some(host) => command.arg("--host").arg(host),
            None => command.arg("--port").arg(port),
        };
    }
    let output = command.output().map_err(|e| e.to_string())?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(15)..].join(
        "
",
    );
    if output.status.success() {
        Ok(tail)
    } else {
        Err(tail)
    }
}

#[cfg(test)]
mod net_tests {
    use super::network_address;

    #[test]
    fn network_ports_are_told_from_serial_ones() {
        assert_eq!(network_address("192.168.1.5"), Some("192.168.1.5"));
        assert_eq!(network_address("tcp:board"), Some("board"));
        assert_eq!(network_address("COM3"), None);
        assert_eq!(network_address("/dev/ttyUSB0"), None);
    }
}
