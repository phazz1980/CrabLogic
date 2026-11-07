//! Main window: menu, toolbar, panels and global shortcuts.

mod chrome;
mod debug;
mod editing;
mod jobs;
mod library;
mod project;
mod selfupdate_ui;
use library::{find_library, is_block_file};

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// How often the editor looks for a new release while it stays open.
const UPDATE_RECHECK: Duration = Duration::from_secs(6 * 3600);

use crab_build::{ArduinoCli, SerialPort};
use crab_core::project::PROJECT_EXT;
use crab_core::{
    BlockPos, BoardConfig, Fragment, Library, LocalValues, PortRef, Project, ProjectConfig,
    ProjectInfo, Severity,
};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText};

use crate::canvas::{self, CanvasState};
use crate::debugview::{self, DebugState};
use crate::dialogs::{self, Dialog};
use crate::doc::Doc;
use crate::job::{Job, JobEvent, JobKind};
use crate::panels;
use crate::selfupdate;
use crate::theme::{self, Theme};
use crate::update::{self, Check, Release};

/// What the master firmware build depends on: project folder (build folder), board with
/// its settings, and the embedded master sketch.
type MasterKey = (PathBuf, u64, u64);

/// Identifies the board and its settings: what, besides libraries, decides the core build.
fn board_hash(board: &impl std::fmt::Debug) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{board:?}").hash(&mut hasher);
    hasher.finish()
}

/// What is selected on the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Block(String),
    /// Several blocks (two or more), selected with Ctrl or a frame.
    Blocks(Vec<String>),
    /// A link, identified by the input it feeds.
    Link(PortRef),
}

impl Selection {
    /// Selection of the given blocks: none, one or a group.
    pub fn of_blocks(mut ids: Vec<String>) -> Option<Self> {
        ids.dedup();
        match ids.len() {
            0 => None,
            1 => ids.pop().map(Selection::Block),
            _ => Some(Selection::Blocks(ids)),
        }
    }

    pub fn blocks(&self) -> &[String] {
        match self {
            Selection::Block(id) => std::slice::from_ref(id),
            Selection::Blocks(ids) => ids,
            Selection::Link(_) => &[],
        }
    }

    pub fn has_block(&self, id: &str) -> bool {
        self.blocks().iter().any(|b| b == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    Log,
    Console,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
    Library,
    Ai,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Ok,
    Warning,
    Error,
}

pub struct LogLine {
    pub level: LogLevel,
    pub text: crate::i18n::LogText,
    /// Block to select when the line is clicked.
    pub block: Option<String>,
    /// Help target (`page#anchor`) that explains the problem, for the "Что это?" link.
    pub help: Option<String>,
}

/// Modal question asked before discarding unsaved changes.
#[derive(Debug, Clone)]
enum Pending {
    Open(PathBuf),
    Close,
}

/// The choice for a block whose copies in the project and in the program differ.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockPick {
    /// Keep the project's copy.
    Keep,
    /// Replace it with the program's copy.
    Replace,
    /// Not chosen yet.
    Undecided,
}

pub struct EditorApp {
    pub lib: Library,
    pub lib_dir: Option<PathBuf>,
    /// `crablogic-web` started from the "Конструктор веб-интерфейса" menu item.
    web_editor: Option<std::process::Child>,
    /// Web widgets of the project as last seen; a change is saved at once for the constructor.
    web_sig: String,
    lib_stamp: Option<SystemTime>,
    lib_checked: Instant,
    /// Modification time of the project file as the editor last read or wrote it; a
    /// different time means another program (MCP server, web designer) changed it.
    disk_stamp: Option<SystemTime>,
    disk_checked: Instant,
    /// The changed-on-disk warning for unsaved work was already shown for this stamp.
    disk_warned: bool,
    pub doc: Option<Doc>,
    pub selection: Option<Selection>,
    /// Active sheet: where blocks added from the library go.
    pub sheet: u32,
    /// Sheets added by the user that have no blocks yet.
    pub extra_sheets: BTreeSet<u32>,
    /// Last tag or variable clicked in the tree and the index of the image shown:
    /// the next click on it shows the next image.
    pub tree_cycle: Option<(String, usize)>,
    pub canvas: CanvasState,
    pub right_tab: RightTab,
    /// Panel sizes and which side panels are expanded; saved in the editor settings.
    pub panels: theme::PanelSizes,
    pub log: Vec<LogLine>,
    pub bottom_tab: BottomTab,
    /// Port monitor, or the console of the debugging session.
    pub console: crate::console::PortConsole,
    /// Open pop-up window (block properties, variable, sheet).
    pub dialog: Option<Dialog>,
    /// The "open project from board" window.
    pub pull: Option<crate::pullview::PullWindow>,
    pub theme: Theme,
    /// Built-in and user themes.
    pub themes: Vec<Theme>,
    pub theme_editor: Option<crate::themeedit::ThemeEditor>,
    /// Open "Block editor" window.
    pub block_editor: Option<crate::blockedit::BlockEditor>,
    pub board_editor: Option<crate::boardedit::BoardEditor>,
    /// Open "Project code" window.
    pub code_view: Option<crate::codeview::CodeView>,
    pub tag_import: Option<crate::tagimport::TagImport>,
    pub help: Option<crate::help::HelpWindow>,
    /// User block waiting for the delete confirmation.
    pub block_delete: Option<String>,
    /// Sheet waiting for the delete confirmation.
    pub sheet_delete: Option<u32>,
    /// Full screen was requested while maximized: wait until the window is restored.
    fs_pending: bool,
    /// Attempts to resize a full-screen window that does not cover the monitor.
    fs_fixes: u8,
    /// Selected upload port; empty means automatic.
    pub port: String,
    pub ports: Vec<SerialPort>,
    port_scan: Option<Receiver<Result<Vec<SerialPort>, String>>>,
    /// Online debugging session, when running.
    pub debug: Option<DebugState>,
    /// Simulation of the schema on this computer, when running.
    pub sim: Option<crate::simview::SimState>,
    /// Build firmware with the debugging module (all builds, precompilation too).
    pub debug_firmware: bool,
    /// Chat with the AI assistant.
    pub ai: crate::ai::AiChat,
    /// Live values published for the AI while debugging.
    ai_live: crate::ai::SharedLive,
    job: Option<Job>,
    /// Build requested while precompilation runs; starts right after it.
    queued: Option<JobKind>,
    /// What the build folder holds a master firmware build of.
    precompiled: Option<MasterKey>,
    /// Last seen master key and when it changed: precompile after a pause.
    seen_rev: Option<(MasterKey, Instant)>,
    pending: Option<Pending>,
    allow_close: bool,
    /// Autosave period in minutes; 0 means off.
    pub autosave_minutes: u32,
    last_autosave: Instant,
    /// Values kept by the program instead of the project file: Wi-Fi passwords
    /// for each SSID and the list of entered SSIDs.
    pub local: LocalValues,
    /// Look for a newer release at startup.
    update_check: bool,
    /// Download a found release in the background; installing stays the user's call.
    auto_update: bool,
    /// Offer beta versions (GitHub pre-releases) as updates.
    update_beta: bool,
    template_sim: bool,
    zig_path: String,
    zig_status: String,
    w2c2_path: String,
    w2c2_status: String,
    /// Running release check; the flag tells whether the user asked for it.
    update_rx: Option<Receiver<(bool, Result<Check, String>)>>,
    /// When the last update check was started (periodic re-check while the editor stays open).
    update_checked: Instant,
    /// Newer release found on GitHub.
    new_release: Option<Release>,
    /// The window with the changes of the newer releases is shown.
    update_window: bool,
    changes_window: bool,
    pub mcp: crate::mcp_host::McpHost,
    pub ai_conf: crate::aiconf::AiConf,
    cores_conf: crate::coresconf::CoresConf,
    report: crate::report::Form,
    pub import: crate::blockio::Import,
    pub board_import: crate::boardio::Import,
    /// User blocks whose revision differs between the project and the program.
    block_diffs: Vec<crab_core::packed::VersionDiff>,
    /// What to do with each block of `block_diffs`.
    block_pick: std::collections::BTreeMap<String, BlockPick>,
    /// Replacing a block also saves the project's copy next to it as `<type>_project`.
    block_keep_copy: bool,
    /// arduino-cli is missing: the window offers to install it.
    cli_prompt: CliPrompt,
    /// Which core packs the startup offer will download (`cores::PACKS` order).
    cli_choice: Vec<bool>,
    /// Update download in progress, or the error of the last attempt.
    self_update: SelfUpdate,
}

enum SelfUpdate {
    Idle(Option<String>),
    Downloading {
        done: Arc<AtomicU64>,
        total: Arc<AtomicU64>,
        rx: Receiver<Result<PathBuf, String>>,
        /// Started by the program, not the button: stop after downloading.
        auto: bool,
    },
    /// Downloaded and unpacked, waiting for the user to restart.
    Ready(PathBuf),
}

/// State of the "arduino-cli not found" offer.
enum CliPrompt {
    Hidden,
    Ask,
    /// arduino-cli is there, the ESP32 core is not.
    AskCore,
    /// The download runs on a worker thread and sends its lines and the final result.
    Installing(CliInstall),
}

/// A running arduino-cli / ESP32 core download: progress shared with the worker thread.
struct CliInstall {
    rx: Receiver<Result<String, String>>,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    last: Arc<Mutex<String>>,
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, project: Option<PathBuf>) -> Self {
        let mut app = Self {
            lib: Library::default(),
            lib_dir: None,
            web_editor: None,
            web_sig: String::new(),
            lib_stamp: None,
            lib_checked: Instant::now(),
            disk_stamp: None,
            disk_checked: Instant::now(),
            disk_warned: false,
            doc: None,
            selection: None,
            sheet: 1,
            extra_sheets: BTreeSet::new(),
            tree_cycle: None,
            canvas: CanvasState::default(),
            right_tab: RightTab::Library,
            panels: theme::saved_panels(),
            log: Vec::new(),
            bottom_tab: BottomTab::Log,
            console: Default::default(),
            dialog: None,
            pull: None,
            theme: theme::light(),
            themes: theme::load_all(),
            theme_editor: None,
            block_editor: None,
            board_editor: None,
            code_view: None,
            tag_import: None,
            help: None,
            block_delete: None,
            sheet_delete: None,
            fs_pending: false,
            fs_fixes: 0,
            port: String::new(),
            ports: Vec::new(),
            port_scan: None,
            debug: None,
            sim: None,
            debug_firmware: false,
            ai: Default::default(),
            ai_live: Default::default(),
            job: None,
            pending: None,
            allow_close: false,
            autosave_minutes: theme::autosave_minutes(),
            last_autosave: Instant::now(),
            queued: None,
            precompiled: None,
            seen_rev: None,
            local: LocalValues::default(),
            update_check: theme::update_check(),
            update_beta: theme::update_beta(),
            template_sim: theme::template_sim(),
            zig_path: theme::zig_path(),
            zig_status: String::new(),
            w2c2_path: theme::w2c2_path(),
            w2c2_status: String::new(),
            auto_update: theme::auto_update(),
            update_rx: None,
            update_checked: Instant::now(),
            new_release: None,
            update_window: false,
            changes_window: false,
            mcp: Default::default(),
            ai_conf: Default::default(),
            cores_conf: Default::default(),
            report: Default::default(),
            import: Default::default(),
            board_import: Default::default(),
            block_diffs: Vec::new(),
            block_pick: std::collections::BTreeMap::new(),
            block_keep_copy: false,
            self_update: SelfUpdate::Idle(None),
            cli_choice: (0..crab_build::cores::PACKS.len())
                .map(|i| i == 0)
                .collect(),
            cli_prompt: match ArduinoCli::locate() {
                Err(_) => CliPrompt::Ask,
                Ok(_) if crab_build::cores::fqbn_available("esp32:esp32:esp32") => {
                    CliPrompt::Hidden
                }
                Ok(_) => CliPrompt::AskCore,
            },
        };
        match LocalValues::open_default() {
            Ok(local) => app.local = local,
            // Kept in memory only, so the broken file is not overwritten.
            Err(e) => app.info(
                LogLevel::Error,
                crate::i18n::msg("Сохранённые пароли не прочитаны: {0}", &[&e]),
            ),
        }
        if let Some(t) =
            theme::saved_choice().and_then(|n| app.themes.iter().find(|t| t.name == n).cloned())
        {
            app.theme = t;
        }
        if let Some(lang) = theme::saved_language().and_then(|c| crate::i18n::Lang::from_code(&c)) {
            crate::i18n::set(lang);
        }
        crate::i18n::install_fonts(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, &app.theme);
        app.scan_ports(&cc.egui_ctx);
        if let Some(text) = selfupdate::take_last_result() {
            let level = if text == "ok" {
                LogLevel::Info
            } else {
                LogLevel::Error
            };
            let text = if text == "ok" {
                crate::i18n::tr("Программа обновлена").to_owned()
            } else {
                crate::i18n::fmt(
                    "Обновление не удалось, возвращена прежняя версия ({0})",
                    &[&text],
                )
            };
            app.info(level, text);
        }
        if app.update_check {
            app.start_update_check(&cc.egui_ctx, false);
        }
        app.lib_dir = find_library(project.as_deref());
        crate::aiconf::apply(&theme::ai_settings());
        app.reload_library();
        // A block or board file given on the command line goes to the import window.
        let (project, block_file) = match project {
            Some(p) if is_block_file(&p) || crate::boardio::is_board_file(&p) => (None, Some(p)),
            other => (other, None),
        };
        // Start with a project: the given one, the last one or a fresh one.
        let last = theme::last_project().filter(|p| crab_core::project::is_project(p));
        match project.or(last) {
            Some(path) => app.open(&path),
            None => app.create_default_project(),
        }
        if let Some(path) = block_file {
            if crate::boardio::is_board_file(&path) {
                crate::boardio::add_files(&mut app, &[path]);
            } else {
                crate::blockio::add_files(&mut app, &[path]);
            }
        }
        app
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, theme: Theme) {
        theme::save_choice(&theme.name);
        theme::apply(ctx, &theme);
        self.theme = theme;
    }

    pub fn info(&mut self, level: LogLevel, text: impl Into<crate::i18n::LogText>) {
        self.log.push(LogLine {
            level,
            text: text.into(),
            block: None,
            help: None,
        });
    }

    /// Sheets to show: with blocks, named, added by the user, and the active one.
    pub fn sheets(&self) -> Vec<u32> {
        let mut sheets: BTreeSet<u32> = self.extra_sheets.clone();
        sheets.insert(1);
        sheets.insert(self.sheet);
        if let Some(doc) = &self.doc {
            let layout = &doc.project.layout;
            sheets.extend(layout.blocks.values().map(|p| p.sheet));
            sheets.extend(layout.sheets.iter().map(|s| s.number));
        }
        sheets.into_iter().collect()
    }

    /// Selects a block and scrolls it into view.
    pub fn focus_block(&mut self, id: &str) {
        self.canvas.focus = Some(id.to_owned());
        if let Some(pos) = self
            .doc
            .as_ref()
            .and_then(|d| d.project.layout.blocks.get(id))
        {
            self.sheet = pos.sheet;
        }
        self.select(Some(Selection::Block(id.to_owned())));
    }

    pub fn select(&mut self, selection: Option<Selection>) {
        self.selection = selection;
    }
}

impl eframe::App for EditorApp {
    /// Only the window geometry is remembered between runs, not egui's own widget state.
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        crate::splash::show(&ctx);
        crab_core::local::editor_beat(self.doc.as_ref().map(|d| d.project.path.as_path()));
        ctx.request_repaint_after(crab_core::local::EDITOR_BEAT);
        crate::textmenu::begin(&ctx);
        self.sync_comment_lang();
        self.poll_job(&ctx);
        self.precompile(&ctx);
        self.poll_debug();
        // Wires of fast 0/1 signals follow the history of the board: redraw about 30 times a second.
        if self.debug.as_ref().is_some_and(|d| d.animating()) {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        self.poll_sim(&ctx);
        self.poll_ai();
        self.poll_console(&ctx);
        self.poll_ports();
        self.poll_update();
        if self.update_check
            && self.update_rx.is_none()
            && self.update_checked.elapsed() >= UPDATE_RECHECK
        {
            self.start_update_check(&ctx, false);
        }
        self.cli_window(&ctx);
        self.mcp
            .set_project(self.doc.as_ref().map(|d| d.project.path.as_path()));
        self.mcp.window(&ctx);
        self.ai_conf.window(&ctx);
        self.cores_conf.window(&ctx);
        self.poll_self_update(&ctx);
        update::changes_window(&ctx, &mut self.changes_window);
        crate::blockio::handle_drops(self, &ctx);
        crate::blockio::window(self, &ctx);
        crate::boardio::handle_drops(self, &ctx);
        crate::boardio::window(self, &ctx);
        if self.report.open {
            let repo = theme::update_repo();
            let board = self.doc.as_ref().map(|d| {
                let b = &d.project.config.board;
                (
                    crate::boardcfg::board_name(b, self.lib.boards()),
                    b.fqbn.clone(),
                )
            });
            let facts = crate::report::Context {
                repo: &repo,
                version: update::CURRENT,
                board: board.as_ref().map(|(n, f)| (*n, f.as_str())),
                log: self.log.iter().map(|l| l.text.to_string()).collect(),
            };
            if let Some(url) = self.report.show(&ctx, &facts) {
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
        }
        if let Some(release) = &self.new_release {
            let install = match &self.self_update {
                SelfUpdate::Idle(e) => update::Install::Idle(e.as_deref()),
                SelfUpdate::Downloading { done, total, .. } => update::Install::Busy(
                    done.load(Ordering::Relaxed),
                    total.load(Ordering::Relaxed),
                ),
                SelfUpdate::Ready(_) => update::Install::Ready,
            };
            let can = selfupdate::install_dir().is_some();
            if update::window(&ctx, release, &mut self.update_window, install, can) {
                if let SelfUpdate::Ready(staged) = &self.self_update {
                    let staged = staged.clone();
                    self.apply_update(&ctx, &staged);
                } else {
                    let url = release.url.clone();
                    self.start_self_update(url, false);
                }
            }
        }
        self.watch_library();
        self.watch_project();
        self.block_diffs_window(&ctx);
        ctx.request_repaint_after(Duration::from_secs(1));
        self.autosave(&ctx);
        self.sync_web_designer();
        self.shortcuts(&ctx);
        self.handle_close(&ctx);
        self.update_title(&ctx);
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        } else {
            // Keep the library watcher ticking even without input.
            ctx.request_repaint_after(Duration::from_secs(2));
        }

        egui::Panel::top("menu").show(ui, |ui| {
            self.title_bar(ui);
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        let mut sizes = self.panels;
        if sizes.log_open {
            sizes.log_height = egui::Panel::bottom("log")
                .resizable(true)
                .default_size(sizes.log_height)
                .show(ui, |ui| panels::bottom(self, ui, &mut sizes.log_open))
                .response
                .rect
                .height();
        } else {
            // One row; a separate id keeps the open height in the panel memory.
            egui::Panel::bottom("log_collapsed")
                .resizable(false)
                .show(ui, |ui| panels::bottom(self, ui, &mut sizes.log_open));
        }
        panels::side_panel(
            ui,
            "tree",
            true,
            &mut sizes.left_open,
            &mut sizes.left_width,
            |ui| panels::left(self, ui),
        );
        panels::side_panel(
            ui,
            "library",
            false,
            &mut sizes.right_open,
            &mut sizes.right_width,
            |ui| panels::right(self, ui),
        );
        // Save once a resize drag is over, not on every frame of it.
        if sizes != self.panels && !ui.input(|i| i.pointer.any_down()) {
            self.panels = sizes;
            theme::save_panels(sizes);
        }
        egui::CentralPanel::default_margins().show(ui, |ui| canvas::show(self, ui));

        dialogs::show(self, &ctx);
        crate::pullview::show(self, &ctx);
        crate::themeedit::show(self, &ctx);
        crate::blockedit::show(self, &ctx);
        crate::boardedit::show(self, &ctx);
        crate::codeview::show(self, &ctx);
        crate::tagimport::show(self, &ctx);
        crate::help::show(self, &ctx);
        crate::blockedit::delete_dialog(self, &ctx);
        crate::canvas::sheet_delete_dialog(self, &ctx);
        let pal = self.theme.palette;
        if let Some(mut s) = self.sim.take() {
            if crate::simview::window(&mut s, &pal, &ctx) {
                self.sim = Some(s);
            }
        }
        if let Some(d) = &mut self.debug {
            debugview::values_window(d, &pal, &ctx);
            debugview::chart_window(d, &pal, &ctx);
            let layout = self.doc.as_ref().map(|doc| &doc.project.layout);
            debugview::diagnostics_window(d, &pal, layout, &ctx);
        }
        self.unsaved_dialog(&ctx);
        crate::textmenu::show(&ctx);
    }
}

/// Keeps helper processes from flashing a console window.
fn hide_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

impl Drop for EditorApp {
    fn drop(&mut self) {
        if let Some(child) = &mut self.web_editor {
            let _ = child.kill();
        }
    }
}
