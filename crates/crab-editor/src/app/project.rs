use super::library::*;
use super::*;

impl EditorApp {
    /// Writes [`EditorApp::local`] to its file, reporting a failure in the log.
    pub fn save_local(&mut self) {
        if let Err(e) = self.local.save() {
            self.info(
                LogLevel::Error,
                crate::i18n::msg("Пароли не сохранены: {0}", &[&e]),
            );
        }
    }

    /// Moves Wi-Fi passwords (and other `local` values) out of the open project
    /// into the program; the next save writes the project file without them.
    pub(super) fn take_local(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        if doc.project.take_local(&self.lib, &mut self.local) {
            doc.dirty = true;
            self.info(
                LogLevel::Info,
                crate::i18n::tr("Пароль Wi-Fi перенесён из файла проекта в настройки программы"),
            );
        }
        self.save_local();
    }

    pub(crate) fn request_open(&mut self, path: PathBuf) {
        if self.doc.as_ref().is_some_and(|d| d.dirty) {
            self.pending = Some(Pending::Open(path));
        } else {
            self.open(&path);
        }
    }

    pub fn open(&mut self, path: &Path) {
        if self.lib_dir.is_none() {
            self.lib_dir = find_library(Some(path));
            self.reload_library();
        }
        // Debugging belongs to the previous project's firmware.
        self.stop_debug();
        self.sim = None;
        self.restore_packed_blocks(path);
        self.block_diffs = if crab_core::project::is_project_file(path) {
            crab_core::packed::version_diffs(path, &self.lib)
        } else {
            Vec::new()
        };
        // The newer copy is preselected; for independent changes nothing is, and when
        // it cannot be told, the program's copy is.
        self.block_pick = self
            .block_diffs
            .iter()
            .map(|d| {
                let pick = match d.newer {
                    crab_core::packed::Newer::Project => BlockPick::Keep,
                    crab_core::packed::Newer::Parallel => BlockPick::Undecided,
                    _ => BlockPick::Replace,
                };
                (d.kind.clone(), pick)
            })
            .collect();
        self.block_keep_copy = self
            .block_diffs
            .iter()
            .any(|d| d.newer == crab_core::packed::Newer::Parallel);
        match Doc::open(path, &self.lib) {
            Ok(mut doc) => {
                self.info(
                    LogLevel::Info,
                    crate::i18n::msg("Открыт проект {0}", &[&path.display()]),
                );
                // Settings of the C++ sketch build are not used by the Rust master: one message,
                // and the next save writes the project without them.
                let dropped = doc.project.drop_sketch_settings();
                if !dropped.is_empty() {
                    doc.dirty = true;
                    self.info(
                        LogLevel::Warning,
                        crate::i18n::fmt(
                            "Проект собирался скетчем Arduino: настройки сборки ({0}) мастеру на Rust не нужны и будут удалены при сохранении. Схема заливается модулями (📦) на мастер-прошивку (⬆)",
                            &[&dropped.join(", ")],
                        ),
                    );
                }
                theme::save_last_project(path);
                self.port = doc.project.config.board.port.clone().unwrap_or_default();
                self.doc = Some(doc);
                self.disk_stamp = self.project_stamp();
                self.disk_warned = false;
                self.selection = None;
                self.sheet = 1;
                self.extra_sheets.clear();
                self.canvas = CanvasState::default();
                self.take_local();
            }
            Err(e) => self.info(LogLevel::Error, e.to_string()),
        }
    }

    /// Opens an example from a copy in the temp folder, so edits and saves never
    /// touch the files in `examples/` (they live in git).
    pub(super) fn open_example(&mut self, name: &str, src: &Path) {
        let dir = std::env::temp_dir().join("crab_examples");
        let dest = dir.join(format!("{name}.crab"));
        let copied = std::fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                if crab_core::project::is_project_file(src) {
                    std::fs::copy(src, &dest)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                } else {
                    let mut project =
                        crab_core::project::Project::load(src).map_err(|e| e.to_string())?;
                    project.path = dest.clone();
                    // Blocks of the example itself (`library/`) travel in the copy and show
                    // as project blocks on open, as for any project file.
                    let blocks = crab_core::packed::pack_block_folders(&src.join("library"));
                    let extra: Vec<(&str, &[u8])> = blocks
                        .iter()
                        .map(|(n, d)| (n.as_str(), d.as_slice()))
                        .collect();
                    project.save_with(&extra).map_err(|e| e.to_string())
                }
            });
        match copied {
            Ok(()) => self.request_open(dest),
            Err(e) => self.info(
                LogLevel::Error,
                crate::i18n::msg("Пример не открыт: {0}", &[&e]),
            ),
        }
    }

    /// Projects from `examples/` next to the library: `(name, path)`, a `.crab` wins over a folder of the same name.
    pub(super) fn example_projects(&self) -> Vec<(String, PathBuf)> {
        let Some(dir) = self
            .lib_dir
            .as_deref()
            .and_then(Path::parent)
            .map(|p| p.join("examples"))
        else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut found: std::collections::BTreeMap<String, PathBuf> = Default::default();
        for path in entries.flatten().map(|e| e.path()) {
            let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
                continue;
            };
            if crab_core::project::is_project_file(&path) {
                found.insert(name, path);
            } else if path.join("project.toml").is_file() {
                found.entry(name).or_insert(path);
            }
        }
        found.into_iter().collect()
    }

    pub(super) fn new_project(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title(crate::i18n::tr("Новый проект"))
            .add_filter(crate::i18n::tr("Проект CrabLogic"), &[PROJECT_EXT])
            .set_file_name(format!("Новый проект.{PROJECT_EXT}"));
        if let Some(dir) = theme::default_projects_dir().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut path) = dialog.save_file() else {
            return;
        };
        if !crab_core::project::is_project_file(&path) {
            path.set_extension(PROJECT_EXT);
        }
        if path.exists() {
            self.request_open(path);
            return;
        }
        self.create_project(&path);
    }

    /// Asks for a project file; an old-format project is opened by its `project.toml`.
    pub(super) fn pick_project(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(crate::i18n::tr("Открыть проект"))
            .add_filter(
                crate::i18n::tr("Проект CrabLogic"),
                &[PROJECT_EXT, crab_core::project::LEGACY_PROJECT_EXT],
            )
            .add_filter(
                crate::i18n::tr("Проект старого формата (project.toml)"),
                &["toml"],
            )
            .pick_file()
        else {
            return;
        };
        let path = match path.parent() {
            Some(dir) if !crab_core::project::is_project_file(&path) => dir.to_path_buf(),
            _ => path,
        };
        self.request_open(path);
    }

    /// Creates "Новый проект N.crab" in the default projects folder and opens it.
    pub(super) fn create_default_project(&mut self) {
        let Some(base) = theme::default_projects_dir() else {
            return;
        };
        if let Err(e) = fs::create_dir_all(&base) {
            self.info(LogLevel::Error, format!("{}: {e}", base.display()));
            return;
        }
        let path = (1..)
            .map(|n| base.join(format!("Новый проект {n}.{PROJECT_EXT}")))
            .find(|p| !p.exists())
            .expect("unbounded range");
        self.create_project(&path);
    }

    /// Writes an empty ESP32 project to `path` and opens it.
    pub(super) fn create_project(&mut self, path: &Path) {
        let name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_owned();
        let project = Project {
            path: path.to_owned(),
            config: ProjectConfig {
                project: ProjectInfo {
                    name,
                    description: String::new(),
                    comment_lang: String::new(),
                    password: String::new(),
                    embed: true,
                },
                board: BoardConfig {
                    fqbn: "esp32:esp32:esp32".into(),
                    model: None,
                    port: None,
                    options: Default::default(),
                },
                uart: Vec::new(),
                retain: Default::default(),
            },
            logic: Default::default(),
            layout: Default::default(),
        };
        if let Err(e) = crab_pdf::save_project(&project, &self.lib) {
            self.info(LogLevel::Error, e.to_string());
            return;
        }
        self.open(path);
    }

    /// Starts the web interface constructor (`crablogic-web`) for the open project and shows
    /// `page` of it in the browser (`/designer`, or `/screens?display=<id>`). The project is
    /// saved first: the constructor edits the file itself.
    pub fn open_web_designer(&mut self, page: &str) {
        let Some(doc) = &self.doc else { return };
        let path = doc.project.path.clone();
        if doc.dirty {
            self.save();
        }
        let running = self
            .web_editor
            .as_mut()
            .is_some_and(|c| matches!(c.try_wait(), Ok(None)));
        if !running {
            let sibling = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("crablogic-web.exe")))
                .filter(|p| p.is_file());
            let mut cmd =
                std::process::Command::new(sibling.unwrap_or_else(|| "crablogic-web".into()));
            cmd.arg(&path);
            if let Some(dir) = &self.lib_dir {
                cmd.arg("--library").arg(dir);
            }
            hide_console(&mut cmd);
            match cmd.spawn() {
                Ok(child) => self.web_editor = Some(child),
                Err(e) => {
                    self.info(
                        LogLevel::Error,
                        crate::i18n::msg(
                            "Не удалось запустить crablogic-web (соберите: cargo build -p crab-web): {0}",
                            &[&e],
                        ),
                    );
                    return;
                }
            }
        }
        // The server needs a moment to start listening; the browser retries on its own.
        std::thread::sleep(Duration::from_millis(500));
        let mut open = std::process::Command::new("cmd");
        open.args(["/C", "start", "", &format!("http://127.0.0.1:8080{page}")]);
        hide_console(&mut open);
        match open.spawn() {
            Ok(_) => self.info(
                LogLevel::Info,
                crate::i18n::tr(if page.starts_with("/screens") {
                    "Конструктор экранов открыт в браузере."
                } else {
                    "Конструктор веб-интерфейса открыт в браузере."
                }),
            ),
            Err(e) => self.info(
                LogLevel::Error,
                crate::i18n::msg("Не удалось открыть браузер: {0}", &[&e]),
            ),
        }
    }

    /// Saves the open project; `false` if it could not be saved.
    pub fn save(&mut self) -> bool {
        let Some(doc) = &mut self.doc else {
            return false;
        };
        match doc.save(&self.lib) {
            Ok(warning) => {
                let path = doc.project.path.display().to_string();
                self.info(LogLevel::Ok, crate::i18n::msg("Сохранено: {0}", &[&path]));
                if let Some(w) = warning {
                    self.info(LogLevel::Warning, w);
                }
                self.disk_stamp = self.project_stamp();
                self.disk_warned = false;
                true
            }
            Err(e) => {
                self.info(LogLevel::Error, e.to_string());
                false
            }
        }
    }

    /// Asks for a new file and saves the project there; the editor then works with
    /// the new file. Also converts an old-format project folder into a `.crab`.
    pub fn save_as(&mut self) {
        let Some(doc) = &self.doc else { return };
        let old = doc.project.path.clone();
        let name = doc.project.config.project.name.clone();
        let mut dialog = rfd::FileDialog::new()
            .set_title(crate::i18n::tr("Сохранить проект как"))
            .add_filter(crate::i18n::tr("Проект CrabLogic"), &[PROJECT_EXT]);
        dialog = match old.parent().filter(|d| d.is_dir()) {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        };
        let file = old
            .file_stem()
            .and_then(|n| n.to_str())
            .map_or(name, str::to_owned);
        let Some(mut path) = dialog
            .set_file_name(format!("{file}.{PROJECT_EXT}"))
            .save_file()
        else {
            return;
        };
        if !crab_core::project::is_project_file(&path) {
            path.set_extension(PROJECT_EXT);
        }
        if let Some(doc) = &mut self.doc {
            doc.project.path = path.clone();
        }
        if self.save() {
            theme::save_last_project(&path);
        } else if let Some(doc) = &mut self.doc {
            // Saving failed: stay with the previous file.
            doc.project.path = old;
        }
    }

    /// Asks for a file and writes the picture of all sheets of the project there.
    pub(super) fn export_pdf(&mut self) {
        let Some(doc) = &self.doc else { return };
        let path = &doc.project.path;
        let stem = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("boards")
            .to_owned();
        let mut dialog = rfd::FileDialog::new()
            .set_title(crate::i18n::tr("Сохранить PDF"))
            .add_filter("PDF", &["pdf"])
            .set_file_name(format!("{stem}.pdf"));
        if let Some(dir) = path.parent().filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(mut dest) = dialog.save_file() else {
            return;
        };
        if dest.extension().is_none() {
            dest.set_extension("pdf");
        }
        let result = crab_pdf::render(&doc.project, &self.lib)
            .map_err(|e| e.to_string())
            .and_then(|pdf| std::fs::write(&dest, pdf).map_err(|e| e.to_string()));
        match result {
            Ok(()) => self.info(
                LogLevel::Ok,
                crate::i18n::msg("PDF сохранён: {0}", &[&dest.display()]),
            ),
            Err(e) => self.info(
                LogLevel::Error,
                crate::i18n::msg("Не удалось сохранить PDF: {0}", &[&e]),
            ),
        }
    }

    /// The web constructor reads the project file, so while it runs, a change of the web
    /// widgets (added, removed, edited) is saved at once instead of waiting for autosave.
    pub(super) fn sync_web_designer(&mut self) {
        let Some(doc) = &self.doc else { return };
        let sig = serde_json::to_string(
            &doc.project
                .logic
                .blocks
                .iter()
                .filter(|b| b.kind.starts_with("web_"))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        if sig == self.web_sig {
            return;
        }
        let first = self.web_sig.is_empty();
        self.web_sig = sig;
        // the constructor may also have been started by hand, so look for its port
        let running = self
            .web_editor
            .as_mut()
            .is_some_and(|c| matches!(c.try_wait(), Ok(None)))
            || std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], 8080)),
                Duration::from_millis(50),
            )
            .is_ok();
        if running && !first && doc.dirty {
            self.save();
        }
    }

    /// Saves a changed project once the autosave period has passed.
    pub(super) fn autosave(&mut self, ctx: &egui::Context) {
        if self.autosave_minutes == 0 {
            return;
        }
        let period = Duration::from_secs(u64::from(self.autosave_minutes) * 60);
        let elapsed = self.last_autosave.elapsed();
        if elapsed < period {
            ctx.request_repaint_after(period - elapsed);
            return;
        }
        self.last_autosave = Instant::now();
        ctx.request_repaint_after(period);
        if self.doc.as_ref().is_some_and(|d| d.dirty) {
            self.save();
        }
    }

    pub fn set_autosave(&mut self, minutes: u32) {
        self.autosave_minutes = minutes;
        self.last_autosave = Instant::now();
        theme::save_autosave_minutes(minutes);
    }
}
