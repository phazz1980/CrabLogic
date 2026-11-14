//! "Report a bug" (`specs/bug-report`): a form that saves a ZIP for the developer. The editor
//! sends nothing itself; the user passes the file on.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use crab_core::report::{self, BoardData, Report};
use crab_core::Project;
use eframe::egui;

/// Newest messages of the editor put into the report.
const RECENT_LINES: usize = 50;

/// What the editor knows when the form is drawn.
pub struct Facts<'a> {
    pub project: Option<&'a Project>,
    /// Name of the board of the open project.
    pub board_name: Option<String>,
    /// Port of the board, when there is one.
    pub port: Option<String>,
    /// Newest messages, oldest first.
    pub recent: Vec<String>,
}

/// What the form tells the editor.
pub enum Event {
    /// The user chose the file: the editor must free the port, then call [`Form::start`].
    Save(PathBuf),
    /// The report is written.
    Done {
        path: PathBuf,
        problems: Vec<String>,
    },
}

/// Result of the background build.
struct Finished {
    path: PathBuf,
    problems: Vec<String>,
}

/// State of the report window.
pub struct Form {
    pub open: bool,
    what: String,
    with_project: bool,
    with_board: bool,
    /// Text of `panic.txt` attached to this report (the form was opened from the crash question).
    panic: Option<String>,
    /// Crash of the previous run, not yet decided on.
    crash_question: Option<String>,
    working: Option<Receiver<Finished>>,
}

impl Default for Form {
    fn default() -> Self {
        Self {
            open: false,
            what: String::new(),
            with_project: false,
            with_board: true,
            panic: None,
            crash_question: None,
            working: None,
        }
    }
}

impl Form {
    /// The form is building a report: the board port is taken.
    pub fn building(&self) -> bool {
        self.working.is_some()
    }

    /// The report will read the board (its checkbox is on).
    pub fn wants_board(&self) -> bool {
        self.with_board
    }

    /// Looks for the note of a crash of the previous run and asks about it.
    pub fn check_crash(&mut self) {
        self.crash_question = crate::editlog::pending_panic();
    }

    /// Draws the windows and the progress; returns what the editor has to do.
    pub fn show(&mut self, ctx: &egui::Context, facts: &Facts) -> Option<Event> {
        self.crash_window(ctx);
        if let Some(Finished { path, problems }) = self.poll(ctx) {
            self.open = false;
            self.what.clear();
            self.panic = None;
            return Some(Event::Done { path, problems });
        }
        if !self.open {
            return None;
        }
        let busy = self.working.is_some();
        let mut save = false;
        let mut open = self.open;
        egui::Window::new(crate::i18n::tr("Сообщить об ошибке"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    ui.label(crate::i18n::tr("Что делали и что пошло не так"));
                    ui.add(
                        egui::TextEdit::multiline(&mut self.what)
                            .desired_rows(5)
                            .desired_width(f32::INFINITY),
                    );
                    if self.panic.is_some() {
                        ui.weak(crate::i18n::tr(
                            "К отчёту приложена запись о прошлом сбое программы.",
                        ));
                    }
                    ui.add_enabled(
                        facts.project.is_some(),
                        egui::Checkbox::new(
                            &mut self.with_project,
                            crate::i18n::tr("Приложить проект (пароли и ключи скрыты)"),
                        ),
                    );
                    let label = match &facts.port {
                        Some(port) => crate::i18n::fmt("Данные платы (порт {0})", &[port]),
                        None => crate::i18n::tr("Данные платы (порт не выбран)").to_owned(),
                    };
                    ui.add_enabled(
                        facts.port.is_some(),
                        egui::Checkbox::new(&mut self.with_board, label),
                    );
                    ui.weak(crate::i18n::tr(
                        "Отладка и монитор порта на время сбора данных платы останавливаются.",
                    ));
                    ui.weak(crate::i18n::tr(
                        "Отчёт — ZIP-файл. Отправьте его разработчику сами (почта, мессенджер). Проверьте проект перед отправкой: скрыты только известные поля паролей.",
                    ));
                });
                ui.horizontal(|ui| {
                    let ready = !self.what.trim().is_empty() && !busy;
                    save = ui
                        .add_enabled(
                            ready,
                            egui::Button::new(crate::i18n::tr("Сохранить отчёт…")),
                        )
                        .clicked();
                    if busy {
                        ui.spinner();
                        ui.label(crate::i18n::tr("Собираю отчёт…"));
                    }
                });
            });
        // The window stays while the report is being built.
        self.open = open || busy;
        if !save {
            return None;
        }
        let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M%S").to_string();
        let mut dialog = rfd::FileDialog::new()
            .set_file_name(report::report_file_name(&stamp))
            .add_filter("ZIP", &["zip"]);
        if let Some(dir) = desktop() {
            dialog = dialog.set_directory(dir);
        }
        dialog.save_file().map(Event::Save)
    }

    /// The question about the crash of the previous run.
    fn crash_window(&mut self, ctx: &egui::Context) {
        if self.crash_question.is_none() {
            return;
        }
        let mut answer = None;
        egui::Window::new(crate::i18n::tr("Прошлый сбой"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(crate::i18n::tr(
                    "В прошлый раз программа завершилась с ошибкой. Сохранить отчёт об ошибке?",
                ));
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("Сохранить отчёт…")).clicked() {
                        answer = Some(true);
                    }
                    if ui.button(crate::i18n::tr("Не надо")).clicked() {
                        answer = Some(false);
                    }
                });
            });
        if let Some(save) = answer {
            let text = self.crash_question.take();
            if save {
                self.panic = text;
                self.open = true;
            }
            // Handled either way: the question is not asked again.
            crate::editlog::archive_panic();
        }
    }

    fn poll(&mut self, ctx: &egui::Context) -> Option<Finished> {
        let rx = self.working.as_ref()?;
        match rx.try_recv() {
            Ok(done) => {
                self.working = None;
                Some(done)
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                None
            }
            Err(TryRecvError::Disconnected) => {
                self.working = None;
                Some(Finished {
                    path: PathBuf::new(),
                    problems: vec![crate::i18n::tr("сбор отчёта прервался").to_owned()],
                })
            }
        }
    }

    /// Builds the report on a worker thread and writes it to `path`.
    pub fn start(
        &mut self,
        path: PathBuf,
        facts: &Facts,
        lib: &crab_core::Library,
        version: &str,
        ctx: &egui::Context,
    ) {
        // Masked here (cheap); the packing waits for the thread.
        let masked = match (self.with_project, facts.project) {
            (true, Some(p)) => {
                let mut copy = p.clone();
                report::mask_secrets(&mut copy, lib);
                Some(copy)
            }
            _ => None,
        };
        let port = if self.with_board {
            facts.port.clone()
        } else {
            None
        };
        let skip = facts.recent.len().saturating_sub(RECENT_LINES);
        let mut r = Report {
            description: self.what.trim().to_owned(),
            app: format!("CrabLogic editor {version}"),
            created: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            project_name: facts.project.map(|p| p.config.project.name.clone()),
            board_name: facts.board_name.clone(),
            master_version: None,
            recent: facts.recent[skip..].to_vec(),
            editor_log: None,
            panic: self.panic.clone(),
            board: None,
            project: None,
            problems: Vec::new(),
        };
        let (tx, rx) = channel();
        self.working = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut problems = Vec::new();
            let log = crate::editlog::read_all();
            r.editor_log = (!log.is_empty()).then_some(log);
            if let Some(project) = masked {
                match project.archive_bytes(&[]) {
                    Ok(bytes) => r.project = Some(bytes),
                    Err(e) => {
                        problems.push(crate::i18n::fmt("не удалось упаковать проект: {0}", &[&e]))
                    }
                }
            }
            if let Some(port) = port {
                let dump = crab_debug::board_report::collect_from_port(&port);
                problems.extend(dump.problems);
                r.master_version = report::master_version(&dump.text);
                r.board = Some(BoardData {
                    port,
                    text: dump.text,
                    coredump: dump.coredump,
                });
            }
            // The problems found so far are listed inside the ZIP too.
            r.problems = problems.clone();
            if let Err(e) = report::write_report(&r, &path) {
                problems.push(e.to_string());
            }
            let _ = tx.send(Finished { path, problems });
            ctx.request_repaint();
        });
    }
}

/// Desktop of the user, or the home folder.
fn desktop() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let home = PathBuf::from(home);
    let desk = home.join("Desktop");
    Some(if desk.is_dir() { desk } else { home })
}

/// Shows the file in the file manager.
pub fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(not(windows))]
    {
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_form_is_closed_and_idle() {
        let f = Form::default();
        assert!(!f.open && !f.building());
    }
}
