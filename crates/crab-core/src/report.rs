//! Bug report: masking of secrets in the project copy and the report ZIP.
//! Pure data code: collecting from the board and the editor lives elsewhere.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::{Error, Library, ParamDef, ParamShow, ParamValue, Project, Result};

/// What replaces a secret value in the report copy.
pub const MASK: &str = "***";

/// Is the parameter a secret (password, key, token)? Only text values can be secret.
/// The block definition decides when it is known (`show = "secret"`, `local`); the name
/// rule covers parameters the definition does not mark.
pub fn is_secret_param(def: Option<&ParamDef>, name: &str, value: &ParamValue) -> bool {
    if !matches!(value, ParamValue::Text(_)) {
        return false;
    }
    if let Some(def) = def {
        if def.show == ParamShow::Secret || def.local.is_some() {
            return true;
        }
    }
    let lower = name.to_lowercase();
    ["password", "passwd", "token", "api_key", "apikey"]
        .iter()
        .any(|word| lower.contains(word))
        || lower.ends_with("_key")
        || lower == "key"
}

/// Replaces every non-empty secret value of the project (block parameters and the
/// board-copy password) with [`MASK`]; returns how many values were masked.
pub fn mask_secrets(project: &mut Project, lib: &Library) -> usize {
    let mut count = 0;
    for block in &mut project.logic.blocks {
        let defs = lib.get(&block.kind).map(|b| &b.def.params);
        for (name, value) in &mut block.params {
            let def = defs.and_then(|d| d.iter().find(|p| &p.name == name));
            if !is_secret_param(def, name, value) {
                continue;
            }
            if matches!(value, ParamValue::Text(t) if !t.is_empty()) {
                *value = ParamValue::Text(MASK.into());
                count += 1;
            }
        }
    }
    let password = &mut project.config.project.password;
    if !password.is_empty() {
        *password = MASK.into();
        count += 1;
    }
    count
}

/// Data read from the board (see `crab-debug`).
pub struct BoardData {
    pub port: String,
    /// Answers of `list`, `info`, `log`, `core info` as text.
    pub text: String,
    pub coredump: Option<Vec<u8>>,
}

pub struct Report {
    pub description: String,
    /// E.g. "CrabLogic editor 2.0.0".
    pub app: String,
    /// Local date and time as text, given by the caller.
    pub created: String,
    pub project_name: Option<String>,
    pub board_name: Option<String>,
    /// From the `master=` line of `info`, see [`master_version`].
    pub master_version: Option<String>,
    /// Last messages of the editor, oldest first.
    pub recent: Vec<String>,
    pub editor_log: Option<Vec<u8>>,
    pub panic: Option<String>,
    pub board: Option<BoardData>,
    /// Already masked `.crab` bytes.
    pub project: Option<Vec<u8>>,
    /// What could not be collected (Russian text).
    pub problems: Vec<String>,
}

/// `crablogic-report-<stamp>.zip`.
pub fn report_file_name(stamp: &str) -> String {
    format!("crablogic-report-{stamp}.zip")
}

/// `master=crab-master-rs 2.0.0` in the answer of `info` gives `2.0.0`.
pub fn master_version(info_text: &str) -> Option<String> {
    let line = info_text
        .lines()
        .find_map(|l| l.trim().strip_prefix("master="))?;
    line.split_whitespace().last().map(str::to_owned)
}

fn cut(line: &str, max: usize) -> String {
    match line.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line.to_owned(),
    }
}

fn markdown(r: &Report) -> String {
    let none = "—";
    let mut s = String::from("# Отчёт об ошибке CrabLogic\n\n## Описание\n\n");
    if r.description.trim().is_empty() {
        s.push_str("(не указано)\n");
    } else {
        s.push_str(r.description.trim());
        s.push('\n');
    }
    s.push_str("\n## Окружение\n\n");
    let mut line = |name: &str, value: &str| s.push_str(&format!("- {name}: {value}\n"));
    line("Программа", &r.app);
    line(
        "ОС",
        &format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    );
    line("Создан", &r.created);
    line("Проект", r.project_name.as_deref().unwrap_or(none));
    line("Плата", r.board_name.as_deref().unwrap_or(none));
    line("Порт", r.board.as_ref().map_or(none, |b| b.port.as_str()));
    line(
        "Версия мастера",
        r.master_version.as_deref().unwrap_or(none),
    );
    if let Some(reset) = r.board.as_ref().and_then(|b| {
        b.text
            .lines()
            .find(|l| l.trim_start().starts_with("reset="))
    }) {
        line("Причина перезапуска", reset.trim());
    }
    let dump = match r.board.as_ref().and_then(|b| b.coredump.as_ref()) {
        Some(d) => format!("приложен (coredump.elf, {} байт)", d.len()),
        None => "нет".to_owned(),
    };
    line("Дамп сбоя", &dump);
    if r.panic.is_some() {
        line("Паника редактора", "приложена (panic.txt)");
    }
    if !r.problems.is_empty() {
        s.push_str("\n## Не удалось собрать\n\n");
        for p in &r.problems {
            s.push_str(&format!("- {p}\n"));
        }
    }
    if !r.recent.is_empty() {
        s.push_str("\n## Последние сообщения\n\n```\n");
        let skip = r.recent.len().saturating_sub(50);
        for m in &r.recent[skip..] {
            s.push_str(&cut(m, 300));
            s.push('\n');
        }
        s.push_str("```\n");
    }
    s
}

/// Writes the report ZIP (through a temporary file, then renamed over `out`).
pub fn write_report(r: &Report, out: &Path) -> Result<()> {
    let fail = |message: String| Error::Report {
        path: out.to_path_buf(),
        message,
    };
    let mut entries: Vec<(&str, Vec<u8>)> = vec![("report.md", markdown(r).into_bytes())];
    if let Some(log) = &r.editor_log {
        entries.push(("editor.log", log.clone()));
    }
    if let Some(p) = &r.panic {
        entries.push(("panic.txt", p.clone().into_bytes()));
    }
    if let Some(b) = &r.board {
        entries.push(("board.txt", b.text.clone().into_bytes()));
        if let Some(d) = &b.coredump {
            entries.push(("coredump.elf", d.clone()));
        }
    }
    if let Some(p) = &r.project {
        entries.push(("project.crab", p.clone()));
    }
    let mut tmp = out.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let file = File::create(&tmp).map_err(|e| fail(e.to_string()))?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in &entries {
        zip.start_file(*name, options)
            .map_err(|e| fail(e.to_string()))?;
        zip.write_all(data).map_err(|e| fail(e.to_string()))?;
    }
    zip.finish()
        .map_err(|e| fail(e.to_string()))?
        .flush()
        .map_err(|e| fail(e.to_string()))?;
    std::fs::rename(&tmp, out).map_err(|e| fail(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block, Layout, Logic, ProjectConfig};
    use std::io::Read;

    fn block(id: &str, kind: &str, params: &[(&str, &str)]) -> Block {
        Block {
            id: id.into(),
            kind: kind.into(),
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), ParamValue::Text((*v).to_owned())))
                .collect(),
            ..Block::default()
        }
    }

    fn unzip_all(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut f = zip.by_index(i).unwrap();
                let mut data = Vec::new();
                f.read_to_end(&mut data).unwrap();
                (f.name().to_owned(), data)
            })
            .collect()
    }

    fn contains(hay: &[u8], needle: &str) -> bool {
        hay.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    #[test]
    fn masking_removes_every_secret_from_the_archive() {
        let lib_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
        let (lib, _) = Library::load(lib_dir).unwrap();
        let config: ProjectConfig = toml::from_str(
            "[project]\nname = \"t\"\npassword = \"BoardPass1\"\n[board]\nfqbn = \"x:y:z\"\n",
        )
        .unwrap();
        let logic = Logic {
            blocks: vec![
                block(
                    "wifi",
                    "wifi",
                    &[
                        ("ssid", "HomeNet"),
                        ("password", "S3cretWiFi!"),
                        ("ap_password", "S3cretAp!"),
                    ],
                ),
                block(
                    "mqtt",
                    "mqtt",
                    &[
                        ("broker", "broker.example.org"),
                        ("password", "S3cretMqtt!"),
                    ],
                ),
                block(
                    "net",
                    "web_net",
                    &[("password", "S3cretWeb!"), ("init_password", "S3cretInit!")],
                ),
            ],
            ..Logic::default()
        };
        let mut project = Project {
            panel: Default::default(),
            panel_layout: Default::default(),
            path: "t.crab".into(),
            config,
            logic,
            layout: Layout::default(),
        };
        let secrets = [
            "S3cretWiFi!",
            "S3cretAp!",
            "S3cretMqtt!",
            "S3cretWeb!",
            "S3cretInit!",
            "BoardPass1",
        ];
        let count = mask_secrets(&mut project, &lib);
        assert_eq!(count, secrets.len());
        let bytes = project.archive_bytes(&[]).unwrap();
        let entries = unzip_all(&bytes);
        assert!(!entries.is_empty());
        for (name, data) in &entries {
            for s in secrets {
                assert!(!contains(data, s), "{s} found in {name}");
            }
        }
        let all: Vec<u8> = entries.into_iter().flat_map(|(_, d)| d).collect();
        assert!(contains(&all, "HomeNet"));
        assert!(contains(&all, "broker.example.org"));
    }

    #[test]
    fn name_rule_ignores_non_text_values() {
        let text = ParamValue::Text("x".into());
        assert!(is_secret_param(None, "api_key", &text));
        assert!(is_secret_param(None, "key", &text));
        assert!(!is_secret_param(None, "secret", &ParamValue::Bool(true)));
        assert!(!is_secret_param(None, "keyboard", &text));
    }

    fn base() -> Report {
        Report {
            description: "не работает".into(),
            app: "test 1".into(),
            created: "2026-11-14 10:00".into(),
            project_name: None,
            board_name: None,
            master_version: None,
            recent: vec!["я".repeat(400)],
            editor_log: None,
            panic: None,
            board: None,
            project: None,
            problems: vec!["нет порта".into()],
        }
    }

    #[test]
    fn report_zip_has_expected_entries() {
        let dir = std::env::temp_dir().join(format!("crab-report-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join(report_file_name("x"));
        write_report(&base(), &out).unwrap();
        let names: Vec<String> = unzip_all(&std::fs::read(&out).unwrap())
            .into_iter()
            .map(|e| e.0)
            .collect();
        assert_eq!(names, ["report.md"]);

        let mut r = base();
        r.editor_log = Some(b"log".to_vec());
        r.panic = Some("boom".into());
        r.project = Some(b"PK".to_vec());
        r.board = Some(BoardData {
            port: "COM3".into(),
            text: "> info\nreset=panic\n".into(),
            coredump: Some(vec![1, 2, 3]),
        });
        write_report(&r, &out).unwrap();
        let entries = unzip_all(&std::fs::read(&out).unwrap());
        let names: Vec<&str> = entries.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(
            names,
            [
                "report.md",
                "editor.log",
                "panic.txt",
                "board.txt",
                "coredump.elf",
                "project.crab"
            ]
        );
        let md = String::from_utf8(entries[0].1.clone()).unwrap();
        assert!(md.contains("reset=panic") && md.contains("3 байт") && md.contains("нет порта"));
        assert!(!md.contains(&"я".repeat(301)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn master_version_is_taken_from_info() {
        assert_eq!(
            master_version("x=1\nmaster=crab-master-rs 2.0.0\n").as_deref(),
            Some("2.0.0")
        );
        assert_eq!(master_version("none"), None);
    }
}
