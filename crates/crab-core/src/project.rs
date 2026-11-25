//! Project file `<name>.crab`: a ZIP archive (opens in any archiver) with
//! `project.toml` (board, settings), `logic.toml` (blocks and links),
//! `web.toml` (the web page: its settings and widgets), `layout.toml`
//! (coordinates, editor only) and `boards.pdf` (picture of all sheets). Old projects were folders of the three TOML files; they are still
//! read and converted on save.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};

use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::panel::PANEL_FILE;
use crate::panel_layout::{PanelLayout, PANEL_LAYOUT_FILE};
use crate::toml_io::{parse_toml, read_toml, to_toml};
use crate::{Error, Layout, Logic, Panel, Result};

/// Extension of project files.
pub const PROJECT_EXT: &str = "crab";
/// Extension of projects saved before the rename; still opened.
pub const LEGACY_PROJECT_EXT: &str = "flpn";
/// Work folder next to the project file: generated sketches and build caches.
pub const WORK_DIR: &str = ".crab";

// Entries of the project archive; an old-format project is a folder of the same files.
pub const PROJECT_FILE: &str = "project.toml";
pub const LOGIC_FILE: &str = "logic.toml";
pub const LAYOUT_FILE: &str = "layout.toml";
/// The web page: the `web_ui` block and the widgets, in the format of `logic.toml`.
/// In memory they are blocks of [`Logic`]; on disk they live apart from the logic.
pub const WEB_FILE: &str = "web.toml";
/// Picture of all sheets, for viewing the schema without the editor.
pub const BOARDS_PDF: &str = "boards.pdf";

/// Blocks that bring the internet to the board, in the order the board prefers them (Ethernet, then
/// Wi-Fi, then GSM). The block of a switched-off provider is not in the project. `gsm` is the
/// future data modem (`specs/gsm-data`): it has no block yet, so it only counts once it exists.
pub const NETWORK_PROVIDERS: [&str; 3] = ["ethernet", "wifi", "gsm"];

/// Blocks of the web page (the `web_ui` host and the widgets) are stored in `web.toml`.
pub fn is_web_kind(kind: &str) -> bool {
    kind.starts_with("web_") || is_screen_kind(kind)
}

/// Blocks of the display constructor (screen widgets `disp_*` and menu items): they live
/// in `web.toml` next to the web widgets, but belong to a display, not to the web page.
pub fn is_screen_kind(kind: &str) -> bool {
    kind.starts_with("disp_") || kind == "menu_item"
}

/// Radio blocks (LoRa): added and removed from the «Радио» section of the left panel, not from
/// the library or the tag tree. The modem has pins but is a device, not an input or an output.
pub const RADIO_KINDS: [&str; 3] = ["lora_e220", "lora_point", "lora_centre"];

/// UART blocks of exchange: added from the «UART» folder of the left panel (they wait in the tree
/// until they are dragged onto a sheet) and stay in the tree when taken off a sheet.
pub const UART_KINDS: [&str; 2] = ["uart_read", "uart_send"];

/// Modbus blocks: added and removed from the «Сеть → Modbus» folder of the left panel, not from the
/// library.
pub const MODBUS_KINDS: [&str; 6] = [
    "modbus_rtu_master",
    "modbus_rtu_slave",
    "modbus_tcp_master",
    "modbus_tcp_slave",
    "modbus_device",
    "modbus_tag",
];

/// A block with pins that is a device and not a tag: no row in the tag tree, no copies, it is
/// deleted with the block, not just taken off the sheet. The modem and the displays (their
/// sections of the left panel list them).
pub fn is_device_kind(kind: &str) -> bool {
    matches!(kind, "lora_e220" | "oled" | "lcd")
}

/// A widget of the web page or of a display screen: it waits in the project tree, its
/// place is set in the constructor and the sheets only show images of it.
pub fn is_widget_kind(kind: &str) -> bool {
    is_web_kind(kind) && kind != "web_ui"
}

/// Project settings: `[project]`, `[board]`, `[[uart]]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub project: ProjectInfo,
    pub board: BoardConfig,
    /// Hardware UARTs used by blocks (`[[uart]]`), set up in the board settings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uart: Vec<crate::uart::UartConfig>,
    /// Flash wear protection of retained variables (`[retain]`).
    #[serde(default, skip_serializing_if = "RetainConfig::is_default")]
    pub retain: RetainConfig,
    /// Cycle period of the sheets (`[run]`).
    #[serde(default, skip_serializing_if = "RunConfig::is_default")]
    pub run: RunConfig,
}

/// Cycle period values the master accepts, in milliseconds.
pub const PERIODS_MS: [u32; 4] = [1, 2, 5, 10];
/// Default cycle period, in milliseconds.
pub const DEFAULT_PERIOD_MS: u32 = 10;

/// How the sheets run on the board (`[run]` of `project.toml`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunConfig {
    /// Period between starts of sheet steps, ms: one of [`PERIODS_MS`].
    pub period_ms: u32,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            period_ms: DEFAULT_PERIOD_MS,
        }
    }
}

impl RunConfig {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Default timing of saving retained variables to flash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetainConfig {
    /// Seconds a value must stay unchanged before it is written.
    pub settle_s: u32,
    /// Minimum seconds between two writes of one variable; 0 = no limit.
    pub period_s: u32,
}

impl Default for RetainConfig {
    fn default() -> Self {
        Self {
            settle_s: 5,
            period_s: 600,
        }
    }
}

impl RetainConfig {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Password of the project copy kept in the board's flash; empty means no protection.
    /// The `.crab` file itself stays readable.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub password: String,
    /// Keep the project copy in the firmware (readable back from the board). On by default.
    #[serde(default = "embed_default", skip_serializing_if = "is_true")]
    pub embed: bool,
}

fn embed_default() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardConfig {
    /// Fully qualified board name (FQBN), e.g. `esp32:esp32:esp32`.
    pub fqbn: String,
    /// Exact board on top of the FQBN when it differs in brought-out pins, e.g.
    /// `esp32c3_supermini` (see [`crate::pins::BOARD_MODELS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Serial port for upload and debugging, e.g. `COM3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    /// Network address of the board (`192.168.1.5` or `host:4040`) for upload over the network.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Old board menu options of the sketch build (dropped on save, see `drop_sketch_settings`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, String>,
}

/// Loaded project: one `.crab` archive next to its work folder `.crab/<name>/`.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// The project file (it may not exist yet for a folder in the old format).
    pub path: PathBuf,
    pub config: ProjectConfig,
    pub logic: Logic,
    /// Empty if there is no `layout.toml` (e.g. the schema was written by hand or by AI).
    pub layout: Layout,
    /// Electrical panel (`panel.toml`); empty if the project has none. It feeds the electrical
    /// documentation only, so it is not part of [`Project::content_hash`].
    pub panel: Panel,
    /// Manual layout of the panel sheets (`panel-layout.toml`); empty if there is none. Like the
    /// panel, it is not part of [`Project::content_hash`].
    pub panel_layout: PanelLayout,
}

impl Project {
    /// The name the user sees: the file (or folder) name. The `name` inside `project.toml` goes
    /// stale when a file is copied or renamed outside the editor, so it is only the fallback.
    pub fn display_name(&self) -> String {
        self.path
            .file_stem()
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty())
            .map_or_else(|| self.config.project.name.clone(), str::to_owned)
    }

    /// Cycle period of the sheets in milliseconds (`[run] period_ms`, 10 by default).
    pub fn period_ms(&self) -> u32 {
        self.config.run.period_ms
    }

    /// Effective flash-saving timing of a retained variable in milliseconds:
    /// (settle, period). Period 0 means no limit.
    pub fn retain_timing(&self, var: &crate::vars::Var) -> (u32, u32) {
        let c = &self.config.retain;
        (
            var.settle_s.unwrap_or(c.settle_s).saturating_mul(1000),
            var.period_s.unwrap_or(c.period_s).saturating_mul(1000),
        )
    }

    /// Ids of the blocks standing on switched-off sheets.
    pub fn disabled_blocks(&self) -> std::collections::HashSet<&str> {
        self.layout
            .blocks
            .iter()
            .filter(|(_, p)| self.layout.is_disabled(p.sheet))
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Indices (in [`Logic::links`]) of the links that survive [`Project::without_disabled`].
    pub fn enabled_links(&self) -> Vec<usize> {
        let off = self.disabled_blocks();
        (0..self.logic.links.len())
            .filter(|&i| {
                let l = &self.logic.links[i];
                !off.contains(l.from.block.as_str()) && !off.contains(l.to.block.as_str())
            })
            .collect()
    }

    /// Whether the project has no network provider (none of the blocks of [`NETWORK_PROVIDERS`]):
    /// the blocks that need the network (see [`Project::network_blocks`]) are then left out of the
    /// build. One provider that is on is enough.
    pub fn network_off(&self) -> bool {
        !self
            .logic
            .blocks
            .iter()
            .any(|b| NETWORK_PROVIDERS.contains(&b.kind.as_str()))
    }

    /// Ids of the blocks that need a network provider: MQTT, Sparkplug, NTP, Modbus TCP and the
    /// web page with its widgets. The providers themselves are not among them.
    pub fn network_blocks(&self) -> Vec<&str> {
        self.logic
            .blocks
            .iter()
            .filter(|b| {
                let k = b.kind.as_str();
                k == "ntp"
                    || k.starts_with("mqtt")
                    || k.starts_with("sparkplug")
                    || k.starts_with("modbus_tcp")
                    || k.starts_with("web_")
            })
            .map(|b| b.id.as_str())
            .collect()
    }

    /// A copy without the blocks of switched-off sheets and the links touching them, and without the
    /// blocks that need a network when the project has none: what is checked and built.
    pub fn without_disabled(&self) -> Project {
        let mut out = self.clone();
        let mut off: Vec<String> = self
            .disabled_blocks()
            .into_iter()
            .map(String::from)
            .collect();
        // A switched-off web page builds nothing: its widgets stay only in the project.
        let web_off = self.logic.blocks.iter().any(|b| {
            b.kind == "web_ui"
                && matches!(
                    b.params.get("enabled"),
                    Some(crate::ParamValue::Bool(false))
                )
        });
        if web_off {
            off.extend(
                self.logic
                    .blocks
                    .iter()
                    .filter(|b| b.kind.starts_with("web_"))
                    .map(|b| b.id.clone()),
            );
        }
        // With no Wi-Fi and no Ethernet the network blocks stay in the project but are not built.
        if self.network_off() {
            off.extend(self.network_blocks().into_iter().map(String::from));
        }
        if off.is_empty() {
            return out;
        }
        out.logic.links = self
            .enabled_links()
            .into_iter()
            .map(|i| self.logic.links[i].clone())
            .filter(|l| !off.contains(&l.from.block) && !off.contains(&l.to.block))
            .collect();
        out.logic.blocks.retain(|b| !off.contains(&b.id));
        out.layout.blocks.retain(|id, _| !off.contains(id));
        out
    }

    /// Loads a project file, or a folder holding one. A folder in the old
    /// three-file format is read too; saving converts it to `<folder>/<folder>.crab`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Self::load_file(path);
        }
        if let Some(file) = find_project_file(path) {
            return Self::load_file(&file);
        }
        if path.join(PROJECT_FILE).is_file() {
            return Self::load_legacy(path);
        }
        Err(Error::NoProject {
            path: path.to_owned(),
        })
    }

    fn load_file(path: &Path) -> Result<Self> {
        let read_err = |source| Error::Read {
            path: path.to_owned(),
            source,
        };
        let file = File::open(path).map_err(read_err)?;
        Self::read_archive(BufReader::new(file), path)
    }

    /// Reads a project from archive bytes (e.g. read back from a board); `path` is the
    /// file the project will be saved to.
    pub fn from_archive_bytes(bytes: &[u8], path: impl AsRef<Path>) -> Result<Self> {
        Self::read_archive(std::io::Cursor::new(bytes), path.as_ref())
    }

    fn read_archive<R: Read + std::io::Seek>(reader: R, path: &Path) -> Result<Self> {
        let read_err = |source| Error::Read {
            path: path.to_owned(),
            source,
        };
        let mut zip = ZipArchive::new(reader).map_err(|e| zip_err(path, e))?;
        let mut entry = |name: &str| -> Result<Option<String>> {
            let mut file = match zip.by_name(name) {
                Ok(f) => f,
                Err(ZipError::FileNotFound) => return Ok(None),
                Err(e) => return Err(zip_err(path, e)),
            };
            let mut text = String::new();
            file.read_to_string(&mut text).map_err(read_err)?;
            Ok(Some(text))
        };
        let inner = |name: &str| path.join(name);
        let config = entry(PROJECT_FILE)?.ok_or_else(|| Error::MissingEntry {
            path: path.to_owned(),
            entry: PROJECT_FILE,
        })?;
        let config = parse_toml(&config, &inner(PROJECT_FILE))?;
        let mut logic: Logic = match entry(LOGIC_FILE)? {
            Some(text) => parse_toml(&text, &inner(LOGIC_FILE))?,
            None => Logic::default(),
        };
        let layout = match entry(LAYOUT_FILE)? {
            Some(text) => parse_toml(&text, &inner(LAYOUT_FILE))?,
            None => Layout::default(),
        };
        if let Some(text) = entry(WEB_FILE)? {
            let web: Logic = parse_toml(&text, &inner(WEB_FILE))?;
            logic.blocks.extend(web.blocks);
        }
        let panel = match entry(PANEL_FILE)? {
            Some(text) => parse_toml(&text, &inner(PANEL_FILE))?,
            None => Panel::default(),
        };
        let panel_layout = match entry(PANEL_LAYOUT_FILE)? {
            Some(text) => parse_toml(&text, &inner(PANEL_LAYOUT_FILE))?,
            None => PanelLayout::default(),
        };
        Ok(Self {
            path: path.to_owned(),
            config,
            logic,
            layout,
            panel,
            panel_layout,
        }
        .migrated())
    }

    /// Builds a project from the texts of its three files kept in memory (the browser
    /// demo has no folders); `name` stands in for the project path.
    pub fn from_sources(name: &str, project: &str, logic: &str, layout: &str) -> Result<Self> {
        let at = |file: &str| Path::new(name).join(file);
        Ok(Self {
            path: at(&format!("{name}.{PROJECT_EXT}")),
            config: parse_toml(project, &at(PROJECT_FILE))?,
            logic: parse_toml(logic, &at(LOGIC_FILE))?,
            layout: parse_toml(layout, &at(LAYOUT_FILE))?,
            panel: Panel::default(),
            panel_layout: PanelLayout::default(),
        }
        .migrated())
    }

    fn load_legacy(dir: &Path) -> Result<Self> {
        let config = read_toml(&dir.join(PROJECT_FILE))?;
        let mut logic: Logic = read_toml(&dir.join(LOGIC_FILE))?;
        let web_path = dir.join(WEB_FILE);
        if web_path.exists() {
            let web: Logic = read_toml(&web_path)?;
            logic.blocks.extend(web.blocks);
        }
        let layout_path = dir.join(LAYOUT_FILE);
        let layout = if layout_path.exists() {
            read_toml(&layout_path)?
        } else {
            Layout::default()
        };
        let panel_path = dir.join(PANEL_FILE);
        let panel = if panel_path.exists() {
            read_toml(&panel_path)?
        } else {
            Panel::default()
        };
        let panel_layout_path = dir.join(PANEL_LAYOUT_FILE);
        let panel_layout = if panel_layout_path.exists() {
            read_toml(&panel_layout_path)?
        } else {
            PanelLayout::default()
        };
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project");
        Ok(Self {
            path: dir.join(format!("{name}.{PROJECT_EXT}")),
            config,
            logic,
            layout,
            panel,
            panel_layout,
        }
        .migrated())
    }

    /// Brings a project written by an older version to the current format.
    fn migrated(mut self) -> Self {
        crate::uart::migrate_legacy(&mut self.config.uart, &mut self.logic);
        migrate_wifi_mode(&mut self.logic);
        migrate_struct_enable(&mut self.logic);
        migrate_threshold_compare(&mut self.logic);
        crate::uart::migrate_uart_send(&mut self.logic);
        self.logic.sync_tags();
        self
    }

    /// Drops the settings only the C++ sketch build used (`[board.options]`, the old board menus:
    /// partition scheme, USB CDC and the like) and returns their names. The Rust master is a prebuilt
    /// image (`specs/rust-only`): the next save writes the project without them.
    pub fn drop_sketch_settings(&mut self) -> Vec<String> {
        let names = self.config.board.options.keys().cloned().collect();
        self.config.board.options.clear();
        names
    }

    /// Writes the project archive with just the schema files.
    pub fn save(&self) -> Result<()> {
        self.save_with(&[])
    }

    /// Writes the project archive: the schema files plus `extra` entries
    /// (name, contents), e.g. [`BOARDS_PDF`].
    ///
    /// The archive is written next to the project and then moved over it, so a
    /// failed save never leaves a broken project. Comments in the TOML files
    /// are not preserved.
    pub fn save_with(&self, extra: &[(&str, &[u8])]) -> Result<()> {
        let tmp = self.path.with_extension(format!("{PROJECT_EXT}.tmp"));
        let write_err = |source| Error::Write {
            path: self.path.clone(),
            source,
        };
        let file = File::create(&tmp).map_err(write_err)?;
        let mut out = BufWriter::new(file);
        self.write_archive(&mut out, extra)?;
        out.flush().map_err(write_err)?;
        drop(out);
        std::fs::rename(&tmp, &self.path).map_err(write_err)
    }

    /// The archive (schema files plus `extra` entries) packed in memory: the copy that goes
    /// into the firmware or a bug report. Equal projects give equal bytes (no timestamps).
    pub fn archive_bytes(&self, extra: &[(&str, &[u8])]) -> Result<Vec<u8>> {
        let mut out = std::io::Cursor::new(Vec::new());
        self.write_archive(&mut out, extra)?;
        Ok(out.into_inner())
    }

    fn write_archive<W: Write + std::io::Seek>(
        &self,
        out: &mut W,
        extra: &[(&str, &[u8])],
    ) -> Result<()> {
        let texts = self.files(true)?;
        let mut zip = ZipWriter::new(out);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let entries = texts
            .iter()
            .map(|(name, text)| (*name, text.as_bytes()))
            .chain(extra.iter().copied());
        for (name, data) in entries {
            zip.start_file(name, options)
                .map_err(|e| zip_err(&self.path, e))?;
            zip.write_all(data).map_err(|source| Error::Write {
                path: self.path.clone(),
                source,
            })?;
        }
        zip.finish().map_err(|e| zip_err(&self.path, e))?;
        Ok(())
    }

    /// The text of the schema files as they are stored: the web page blocks go to
    /// `web.toml` (written only when there are any), the rest of the logic stays in
    /// `logic.toml`. `with_layout` adds `layout.toml`, `panel.toml` (when the panel is not
    /// empty) at the end: the files of the archive that do not count for the hash.
    fn files(&self, with_layout: bool) -> Result<Vec<(&'static str, String)>> {
        let inner = |name: &str| self.path.join(name);
        let (web, rest): (Vec<_>, Vec<_>) = self
            .logic
            .blocks
            .iter()
            .cloned()
            .partition(|b| is_web_kind(&b.kind));
        let logic = Logic {
            blocks: rest,
            ..self.logic.clone()
        };
        let mut files = vec![
            (PROJECT_FILE, to_toml(&self.config, &inner(PROJECT_FILE))?),
            (LOGIC_FILE, to_toml(&logic, &inner(LOGIC_FILE))?),
        ];
        if !web.is_empty() {
            let page = Logic {
                blocks: web,
                ..Logic::default()
            };
            files.push((WEB_FILE, to_toml(&page, &inner(WEB_FILE))?));
        }
        if with_layout {
            files.push((LAYOUT_FILE, to_toml(&self.layout, &inner(LAYOUT_FILE))?));
            // Like the layout, the panel does not change the firmware: it stays out of the hash.
            if !self.panel.is_empty() {
                files.push((PANEL_FILE, to_toml(&self.panel, &inner(PANEL_FILE))?));
            }
            // Written without the entries of devices the panel no longer has; a layout that is
            // empty after that is not written at all.
            let mut panel_layout = self.panel_layout.clone();
            panel_layout.prune(&self.panel);
            if !panel_layout.is_empty() {
                files.push((
                    PANEL_LAYOUT_FILE,
                    to_toml(&panel_layout, &inner(PANEL_LAYOUT_FILE))?,
                ));
            }
        }
        Ok(files)
    }

    /// Short identity of the project's logic and settings (`project.toml` and
    /// `logic.toml`; coordinates in `layout.toml` do not count). The firmware carries
    /// the same value, so the editor can tell whether the board runs this project.
    pub fn content_hash(&self) -> Result<[u8; 8]> {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        for (name, text) in self.files(false)? {
            hasher.update((text.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
            hasher.update(text.as_bytes());
        }
        let mut hash = [0u8; 8];
        hash.copy_from_slice(&hasher.finalize()[..8]);
        Ok(hash)
    }

    /// Folder the project file lives in.
    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new(""))
    }

    /// Project name taken from the file name.
    pub fn file_stem(&self) -> &str {
        self.path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("project")
    }

    /// Generated sketch and build cache: `<dir>/.crab/<file name>/`, so several
    /// projects can share a folder.
    pub fn work_dir(&self) -> PathBuf {
        self.dir().join(WORK_DIR).join(self.file_stem())
    }
}

/// The only `.crab` file in `dir`, if there is exactly one.
pub fn find_project_file(dir: &Path) -> Option<PathBuf> {
    let mut files = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| is_project_file(p));
    let first = files.next()?;
    files.next().is_none().then_some(first)
}

/// Whether `path` has the project file extension.
pub fn is_project_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        e.eq_ignore_ascii_case(PROJECT_EXT) || e.eq_ignore_ascii_case(LEGACY_PROJECT_EXT)
    })
}

/// Whether `path` is something [`Project::load`] can open.
pub fn is_project(path: &Path) -> bool {
    if path.is_dir() {
        find_project_file(path).is_some() || path.join(PROJECT_FILE).is_file()
    } else {
        is_project_file(path) && path.is_file()
    }
}

fn zip_err(path: &Path, source: ZipError) -> Error {
    Error::Archive {
        path: path.to_owned(),
        source,
    }
}

/// The enable input of `struct_pack` was called `go` and stood last; it is `en` and stands first.
fn migrate_struct_enable(logic: &mut Logic) {
    let packs: Vec<String> = logic
        .blocks
        .iter()
        .filter(|b| b.kind == crate::struct_blocks::PACK)
        .map(|b| b.id.clone())
        .collect();
    for link in &mut logic.links {
        if packs.contains(&link.to.block) && link.to.port == "go" {
            link.to.port = crate::struct_blocks::GO.into();
        }
    }
    for block in logic
        .blocks
        .iter_mut()
        .filter(|b| b.kind == crate::struct_blocks::PACK)
    {
        if let Some(v) = block.consts.remove("go") {
            block.consts.insert(crate::struct_blocks::GO.into(), v);
        }
    }
}

/// The Wi-Fi block took its mode from the input when the check box `use_mode`
/// was on; now that is the choice `wifi_mode = 3`.
fn migrate_wifi_mode(logic: &mut Logic) {
    for block in logic.blocks.iter_mut().filter(|b| b.kind == "wifi") {
        if let Some(use_mode) = block.params.remove("use_mode") {
            if use_mode == crate::ParamValue::Bool(true) {
                block
                    .params
                    .insert("wifi_mode".into(), crate::ParamValue::Int(3));
            }
        }
    }
}

/// The blocks `greater` and `less` (an input against a threshold parameter `b`) are the comparator
/// with the operation `>` / `<` and the threshold as a constant on its input `b`.
pub fn migrate_threshold_compare(logic: &mut Logic) {
    for block in logic
        .blocks
        .iter_mut()
        .filter(|b| b.kind == "greater" || b.kind == "less")
    {
        let op = if block.kind == "greater" { 4 } else { 2 };
        block.kind = "compare".into();
        block.params.insert("op".into(), crate::ParamValue::Int(op));
        let threshold = block.params.remove("b");
        // A threshold taken from a link or a variable stays there: only a plain value moves.
        if !block.param_in.remove("b") && !block.bound.contains_key("b") {
            block.consts.insert(
                "b".into(),
                threshold.unwrap_or(crate::ParamValue::Float(0.0)),
            );
        }
    }
}

/// A project hash as 16 hex digits.
pub fn hash_hex(hash: &[u8; 8]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_blocks_become_the_comparator() {
        let mut logic: Logic = toml::from_str(
            r#"
[[block]]
id = "hot"
type = "greater"
b = 30.0

[[block]]
id = "low"
type = "less"

[[block]]
id = "linked"
type = "less"
param_in = ["b"]
"#,
        )
        .unwrap();
        migrate_threshold_compare(&mut logic);
        let hot = logic.block("hot").unwrap();
        assert_eq!(hot.kind, "compare");
        assert_eq!(hot.params.get("op"), Some(&crate::ParamValue::Int(4)));
        assert_eq!(hot.consts.get("b"), Some(&crate::ParamValue::Float(30.0)));
        assert!(!hot.params.contains_key("b"));
        let low = logic.block("low").unwrap();
        assert_eq!(low.params.get("op"), Some(&crate::ParamValue::Int(2)));
        assert_eq!(low.consts.get("b"), Some(&crate::ParamValue::Float(0.0)));
        let linked = logic.block("linked").unwrap();
        assert!(linked.consts.is_empty() && linked.param_in.is_empty());
    }

    #[test]
    fn sketch_settings_are_gone_after_a_save() {
        let dir = std::env::temp_dir().join(format!("crab_sketch_opts_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("project.toml"),
            "[project]
name = \"old\"

[board]
fqbn = \"esp32:esp32:esp32c3\"

             [board.options]
CDCOnBoot = \"cdc\"
PartitionScheme = \"huge_app\"
",
        )
        .unwrap();
        std::fs::write(dir.join("logic.toml"), "").unwrap();
        let mut project = Project::load(&dir).unwrap();
        assert_eq!(
            project.drop_sketch_settings(),
            ["CDCOnBoot", "PartitionScheme"]
        );
        project.save().unwrap();
        let again = Project::load(&project.path).unwrap();
        assert!(again.config.board.options.is_empty());
        assert_eq!(again.config.board.fqbn, "esp32:esp32:esp32c3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wifi_use_mode_becomes_mode_choice() {
        let mut logic: Logic = toml::from_str(
            r#"
[[block]]
id = "on"
type = "wifi"
ssid = "net"
use_mode = true
wifi_mode = 1

[[block]]
id = "off"
type = "wifi"
ssid = "net"
use_mode = false
wifi_mode = 2
"#,
        )
        .unwrap();
        migrate_wifi_mode(&mut logic);
        let on = &logic.blocks[0].params;
        assert_eq!(on.get("wifi_mode"), Some(&crate::ParamValue::Int(3)));
        assert!(!on.contains_key("use_mode"));
        let off = &logic.blocks[1].params;
        assert_eq!(off.get("wifi_mode"), Some(&crate::ParamValue::Int(2)));
        assert!(!off.contains_key("use_mode"));
    }

    #[test]
    fn run_period_is_saved_only_when_not_default() {
        let dir = std::env::temp_dir().join(format!("crab-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = "[project]
name = \"t\"
[board]
fqbn = \"x:y:z\"
";
        let mut config: ProjectConfig = toml::from_str(src).unwrap();
        assert_eq!(config.run.period_ms, 10);
        assert_eq!(
            toml::to_string(&config).unwrap(),
            toml::to_string(&toml::from_str::<ProjectConfig>(src).unwrap()).unwrap()
        );
        assert!(!toml::to_string(&config).unwrap().contains("[run]"));
        config.run.period_ms = 2;
        let project = Project {
            panel: Default::default(),
            panel_layout: Default::default(),
            path: dir.join("t.crab"),
            config,
            logic: Logic::default(),
            layout: Layout::default(),
        };
        assert!(toml::to_string(&project.config).unwrap().contains("[run]"));
        project.save().unwrap();
        let again = Project::load(&project.path).unwrap();
        assert_eq!(again.period_ms(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_roundtrip_keeps_extra_entries() {
        let dir = std::env::temp_dir().join(format!("crab-project-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config: ProjectConfig =
            toml::from_str("[project]\nname = \"t\"\n[board]\nfqbn = \"x:y:z\"\n").unwrap();
        let project = Project {
            panel: Default::default(),
            panel_layout: Default::default(),
            path: dir.join("t.crab"),
            config,
            logic: Logic::default(),
            layout: Layout::default(),
        };
        project.save_with(&[(BOARDS_PDF, b"%PDF")]).unwrap();
        assert_eq!(Project::load(&dir).unwrap(), project);
        let mut zip = ZipArchive::new(File::open(&project.path).unwrap()).unwrap();
        assert!(zip.by_name(BOARDS_PDF).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn panel_is_stored_in_the_archive_only_when_not_empty() {
        use crate::panel::{ContactForm, Device, DeviceKind};
        let dir = std::env::temp_dir().join(format!("crab-panel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config: ProjectConfig =
            toml::from_str("[project]\nname = \"t\"\n[board]\nfqbn = \"x:y:z\"\n").unwrap();
        let mut project = Project {
            panel: Default::default(),
            panel_layout: Default::default(),
            path: dir.join("t.crab"),
            config,
            logic: Logic::default(),
            layout: Layout::default(),
        };
        let has_panel = |path: &Path| {
            let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
            let found = zip.by_name(PANEL_FILE).is_ok();
            found
        };
        project.save().unwrap();
        assert!(!has_panel(&project.path));

        let hash = project.content_hash().unwrap();
        project.panel.devices.push(Device {
            id: "K1".into(),
            kind: DeviceKind::Contactor,
            via: String::new(),
            rating: "230 В, 9 А".into(),
            part: String::new(),
            bind: "pump.out".into(),
            load: "M1".into(),
            poles: None,
            of: String::new(),
            form: ContactForm::default(),
            pins: String::new(),
            from: String::new(),
            free: false,
            series: String::new(),
            conductors: Vec::new(),
        });
        project.save().unwrap();
        assert!(has_panel(&project.path));
        assert_eq!(Project::load(&project.path).unwrap(), project);
        // The panel does not change the firmware, so the board still runs "this" project.
        assert_eq!(project.content_hash().unwrap(), hash);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_folder_with_panel_toml_is_read() {
        let dir = std::env::temp_dir().join(format!("crab-panel-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(PROJECT_FILE),
            "[project]\nname = \"t\"\n[board]\nfqbn = \"x:y:z\"\n",
        )
        .unwrap();
        std::fs::write(dir.join(LOGIC_FILE), "").unwrap();
        std::fs::write(
            dir.join(PANEL_FILE),
            "supply = \"dc24\"\n[[device]]\nid = \"G1\"\nkind = \"psu\"\n",
        )
        .unwrap();
        let project = Project::load(&dir).unwrap();
        assert_eq!(project.panel.devices.len(), 1);
        assert_eq!(project.panel.supply, crate::Supply::Dc24);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn board_host_roundtrips_and_is_skipped_when_unset() {
        let dir = std::env::temp_dir().join(format!("crab-host-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config: ProjectConfig = toml::from_str(
            "[project]
name = \"t\"
[board]
fqbn = \"x:y:z\"
",
        )
        .unwrap();
        assert_eq!(config.board.host, None);
        assert!(!toml::to_string(&config).unwrap().contains("host"));
        config.board.host = Some("192.168.1.50:4040".into());
        let project = Project {
            panel: Default::default(),
            panel_layout: Default::default(),
            path: dir.join("t.crab"),
            config,
            logic: Logic::default(),
            layout: Layout::default(),
        };
        project.save().unwrap();
        let again = Project::load(&project.path).unwrap();
        assert_eq!(
            again.config.board.host.as_deref(),
            Some("192.168.1.50:4040")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_hash_ignores_layout_only() {
        let config: ProjectConfig = toml::from_str(
            "[project]
name = \"t\"
[board]
fqbn = \"x:y:z\"
",
        )
        .unwrap();
        let project = Project {
            panel: Default::default(),
            panel_layout: Default::default(),
            path: PathBuf::from("t.crab"),
            config,
            logic: Logic::default(),
            layout: Layout::default(),
        };
        let hash = project.content_hash().unwrap();
        assert_eq!(hash, project.content_hash().unwrap());

        let mut moved = project.clone();
        moved.layout.sheets.push(Default::default());
        assert_eq!(moved.content_hash().unwrap(), hash);

        let mut renamed = project.clone();
        renamed.config.project.name = "other".into();
        assert_ne!(renamed.content_hash().unwrap(), hash);

        let mut edited = project.clone();
        edited.logic.blocks.push(
            toml::from_str(
                "id = \"a\"
type = \"wifi\"
",
            )
            .unwrap(),
        );
        assert_ne!(edited.content_hash().unwrap(), hash);
    }
}
