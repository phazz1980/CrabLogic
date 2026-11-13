//! `crablogic-web` — the web constructor of the ESP32 web interface (`/`, also `/designer`).
//! A tiny local HTTP server (std only): it serves the page and forwards `POST /api/<command>` to the same command set
//! that the AI chat and the MCP server use, so every edit is validated and saved
//! to the project file exactly as there.
//!
//! Usage: `crablogic-web <project> [--library <dir>] [--port <n>]`; open
//! `http://127.0.0.1:<port>/` (default 8080). Listens on the loopback only.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crab_ai::folder::FolderContext;
use serde_json::{json, Value};

const DESIGNER: &str = include_str!("designer.html");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_PORT: u16 = 8080;
/// Largest request body accepted (commands are small JSON objects).
const MAX_BODY: usize = 1 << 20;
/// The page pings the server every few seconds; with no request for this long the
/// server exits, so it never outlives the browser tab (or the editor that started it).
const IDLE_LIMIT: Duration = Duration::from_secs(300);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("crablogic-web: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut project: Option<PathBuf> = None;
    let mut library: Option<PathBuf> = std::env::var_os("CRAB_LIBRARY").map(PathBuf::from);
    let mut port = DEFAULT_PORT;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--library" => library = Some(args.next().ok_or("после --library нужна папка")?.into()),
            "--port" => {
                port = args
                    .next()
                    .and_then(|p| p.parse().ok())
                    .ok_or("после --port нужен номер порта")?
            }
            _ => project = Some(a.into()),
        }
    }
    let project =
        project.ok_or("использование: crablogic-web <проект.crab> [--library папка] [--port N]")?;
    let library = library
        .or_else(|| find_library(&project))
        .ok_or("не найдена папка библиотеки блоков library/; укажите --library")?;
    let mut ctx = FolderContext::open(&project, &library)?;

    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("не удалось занять порт {port}: {e}"))?;
    println!(
        "Конструктор веб-интерфейса: http://127.0.0.1:{port}/  (проект {})",
        project.display()
    );
    let started = Instant::now();
    let last_seen = std::sync::Arc::new(AtomicU64::new(0));
    let watch = last_seen.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(10));
        let idle = started
            .elapsed()
            .saturating_sub(Duration::from_secs(watch.load(Ordering::Relaxed)));
        if idle > IDLE_LIMIT {
            println!("crablogic-web: нет запросов {} с, выход.", idle.as_secs());
            std::process::exit(0);
        }
    });
    for stream in listener.incoming().flatten() {
        last_seen.store(started.elapsed().as_secs(), Ordering::Relaxed);
        // One user: requests are handled one at a time, edits never race.
        if let Err(e) = handle(stream, &mut ctx, port) {
            eprintln!("crablogic-web: {e}");
        }
    }
    Ok(())
}

struct Request {
    method: String,
    path: String,
    host: String,
    content_type: String,
    body: Vec<u8>,
}

fn read_request(stream: &TcpStream) -> Result<Request, String> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or("пустой запрос")?.to_owned();
    let path = parts.next().ok_or("нет пути")?.to_owned();
    let (mut host, mut content_type, mut length) = (String::new(), String::new(), 0usize);
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).map_err(|e| e.to_string())?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let v = v.trim();
            match k.to_ascii_lowercase().as_str() {
                "host" => host = v.to_owned(),
                "content-type" => content_type = v.to_ascii_lowercase(),
                "content-length" => length = v.parse().map_err(|_| "плохой Content-Length")?,
                _ => {}
            }
        }
    }
    if length > MAX_BODY {
        return Err("слишком большой запрос".into());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(Request {
        method,
        path,
        host,
        content_type,
        body,
    })
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn handle(mut stream: TcpStream, ctx: &mut FolderContext, port: u16) -> Result<(), String> {
    let req = read_request(&stream)?;
    // Only the loopback names: a foreign page cannot reach the editor via DNS rebinding.
    let local = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if !local.contains(&req.host) {
        respond(&mut stream, "403 Forbidden", "text/plain", b"forbidden");
        return Ok(());
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/" | "/designer") => respond(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            DESIGNER.replace("{{VERSION}}", VERSION).as_bytes(),
        ),
        ("GET", "/ui_page.html") => respond(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            crab_blocks::device_page().as_bytes(),
        ),
        ("POST", path) if path.starts_with("/api/") => {
            // A plain cross-site form cannot send JSON, so this blocks forged posts.
            if !req.content_type.starts_with("application/json") {
                respond(
                    &mut stream,
                    "415 Unsupported Media Type",
                    "text/plain",
                    b"json only",
                );
                return Ok(());
            }
            let args: Value = if req.body.is_empty() {
                json!({})
            } else {
                serde_json::from_slice(&req.body).map_err(|e| format!("плохой JSON: {e}"))?
            };
            let answer = match ctx
                .refresh()
                .and_then(|()| crab_ai::call(ctx, &path["/api/".len()..], &args))
            {
                Ok(v) => json!({"ok": true, "result": v}),
                Err(e) => json!({"ok": false, "error": e}),
            };
            respond(
                &mut stream,
                "200 OK",
                "application/json; charset=utf-8",
                answer.to_string().as_bytes(),
            );
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", b"not found"),
    }
    Ok(())
}

/// `library/` in the project folder or its parents, then next to the executable.
fn find_library(project: &Path) -> Option<PathBuf> {
    let starts = [
        std::path::absolute(project).ok(),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf)),
    ];
    starts
        .into_iter()
        .flatten()
        .flat_map(|s| s.ancestors().map(|a| a.join("library")).collect::<Vec<_>>())
        .find(|d| d.is_dir())
}

#[cfg(test)]
mod tests {

    #[test]
    fn preview_page_is_the_one_from_the_block_code() {
        let page = crab_blocks::device_page();
        assert!(page.starts_with("<!DOCTYPE html>"));
        assert!(page.contains("/ui/layout") && page.contains("/ui/state"));
    }
}

#[cfg(test)]
mod designer_tests {
    use super::*;

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let target = to.join(e.file_name());
            if e.path().is_dir() {
                copy_dir(&e.path(), &target);
            } else {
                std::fs::copy(e.path(), target).unwrap();
            }
        }
    }

    /// The commands the designer page sends for displays and menus: widgets and items
    /// land in `web.toml`, the menu variable is created with `retain`, all survive a reopen.
    #[test]
    fn display_commands_of_the_designer_are_saved() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = std::env::temp_dir().join(format!("crab-web-disp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&root.join("examples/blink"), &dir);
        let mut ctx = FolderContext::open(&dir, &root.join("library")).unwrap();
        let mut call = |name: &str, args: Value| {
            crab_ai::call(&mut ctx, name, &args).unwrap_or_else(|e| panic!("{name}: {e}"))
        };
        call("add_block", json!({"type": "oled", "id": "scr"}));
        call(
            "add_block",
            json!({"type": "disp_value", "id": "value", "params":
                {"display": "scr", "screen": 2, "x": 4, "y": 8, "w": 48, "h": 8}}),
        );
        call(
            "add_block",
            json!({"type": "menu", "id": "menu", "params": {"display": "scr"}}),
        );
        call(
            "add_block",
            json!({"type": "menu_item", "id": "sub", "params":
                {"menu": "menu", "parent": "", "order": 0, "kind": "submenu", "caption": "Настройки"}}),
        );
        call(
            "add_block",
            json!({"type": "menu_item", "id": "sp", "params":
                {"menu": "menu", "parent": "sub", "order": 0, "kind": "number", "caption": "Уставка"}}),
        );
        call(
            "set_variable",
            json!({"name": "sp_val", "type": "float", "retain": true}),
        );
        call(
            "set_params",
            json!({"id": "sp", "params": {"var": "sp_val"}}),
        );
        call(
            "set_params",
            json!({"id": "sp", "params": {"order": 3, "parent": ""}}),
        );

        let schema = call("get_schema", json!({}));
        let blocks = schema["logic"]["block"].as_array().unwrap();
        let find = |id: &str| blocks.iter().find(|b| b["id"] == id).unwrap().clone();
        assert_eq!(find("value")["x"], 4);
        assert_eq!(find("value")["screen"], 2);
        assert_eq!(find("sp")["var"], "sp_val");
        assert_eq!(find("sp")["order"], 3);
        let var = schema["logic"]["var"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == "sp_val")
            .unwrap();
        assert_eq!(var["retain"], true);

        // A folder project is saved as `<folder>/<name>.crab`; the screen blocks live in its `web.toml`.
        let archive = dir.join(format!(
            "{}.crab",
            dir.file_name().unwrap().to_string_lossy()
        ));
        let again = FolderContext::open(&archive, &root.join("library")).unwrap();
        let p = crab_ai::Context::project(&again);
        assert!(p.logic.block("sp").is_some() && p.logic.block("value").is_some());
        assert!(p.logic.var("sp_val").is_some_and(|v| v.retain));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn page_exposes_the_font_hook_and_menu_tab() {
        assert!(DESIGNER.contains("window.crabFont"));
        // ids of new blocks are chosen from a fresh schema (a stale poll once produced duplicates)
        assert!(DESIGNER.contains("await freshSchema();") && DESIGNER.contains("seen !== epoch"));
        assert!(DESIGNER.contains("renderMenu") && DESIGNER.contains("disp_bar"));
    }
}
