//! `wamrc` (the WAMR AOT compiler): finding it, downloading it on first use and running it.
//!
//! It is not shipped with the program: the pinned release is fetched into
//! `<program folder>/tools/wamrc-<version>/` and checked against a SHA-256.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

/// WAMR release the compiler is taken from (must match the runtime in the master firmware).
pub const WAMRC_VERSION: &str = "2.4.5";
/// SHA-256 of `wamrc-<version>-x86_64-windows-2022.zip`.
const WAMRC_ZIP_SHA256: &str = "a913cda13f7d8aa6c3d77229d3f9c268a8714c5ce86be3e13b39cedb07ea958e";
/// Name of the compiler inside the archive and in the tool folder.
const EXE_NAME: &str = "wamrc.exe";
/// Environment variable with a path to a hand-installed compiler.
const ENV_VAR: &str = "CRAB_WAMRC";

/// CPU a module is compiled for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AotTarget {
    /// ESP32-C3; Xtensa comes later.
    Riscv32Imc,
}

impl AotTarget {
    /// wamrc flags selecting the CPU (same as `firmware/master-rs/build.rs`).
    pub fn args(self) -> &'static [&'static str] {
        match self {
            AotTarget::Riscv32Imc => &[
                "--target=riscv32",
                "--target-abi=ilp32",
                "--cpu=generic-rv32",
                "--cpu-features=+m,+c,-a,-f,-d",
                "--xip",
                "--disable-simd",
                "--disable-ref-types",
                "--bounds-checks=1",
            ],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WamrcError {
    #[error("отменено")]
    Cancelled,
    #[error(
        "не удалось скачать wamrc: {0}. Проверьте подключение к интернету или скачайте wamrc \
         вручную со страницы релиза WAMR (github.com/bytecodealliance/wasm-micro-runtime/releases) \
         и укажите путь к нему в настройках (или в переменной среды CRAB_WAMRC)"
    )]
    Download(String),
    #[error("контрольная сумма скачанного wamrc не совпала, файл отброшен")]
    Checksum,
    #[error("архив wamrc повреждён или не содержит wamrc.exe: {0}")]
    BadArchive(String),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("не удалось запустить wamrc ({}): {source}", path.display())]
    Spawn {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("wamrc завершился с ошибкой:\n{0}")]
    Failed(String),
    #[error(
        "автоматическая загрузка wamrc поддерживается только в Windows x64: скачайте wamrc \
         вручную и укажите путь к нему в настройках (или в переменной среды CRAB_WAMRC)"
    )]
    UnsupportedOs,
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> WamrcError + '_ {
    move |source| WamrcError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `<program folder>/tools/wamrc-<version>/` whether or not it exists.
pub fn tool_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(
        exe.parent()?
            .join("tools")
            .join(format!("wamrc-{WAMRC_VERSION}")),
    )
}

/// The compiler to use, without downloading: `manual` (path from the settings) if it is an existing
/// file, else env var `CRAB_WAMRC` if it is an existing file, else the cached exe in `tool_dir()`.
pub fn locate(manual: Option<&Path>) -> Option<PathBuf> {
    let env = std::env::var_os(ENV_VAR).map(PathBuf::from);
    locate_in(manual, env.as_deref(), tool_dir().as_deref())
}

fn locate_in(
    manual: Option<&Path>,
    env_value: Option<&Path>,
    dir: Option<&Path>,
) -> Option<PathBuf> {
    manual
        .filter(|p| p.is_file())
        .or_else(|| env_value.filter(|p| p.is_file()))
        .map(Path::to_path_buf)
        .or_else(|| dir.map(|d| d.join(EXE_NAME)).filter(|p| p.is_file()))
}

/// Downloads the pinned release into `tool_dir()` and returns the path of the exe.
pub fn install(
    cancel: &AtomicBool,
    on_line: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, WamrcError> {
    if !cfg!(windows) {
        return Err(WamrcError::UnsupportedOs);
    }
    let dir =
        tool_dir().ok_or_else(|| WamrcError::Download("не найдена папка программы".to_string()))?;
    let url = format!(
        "https://github.com/bytecodealliance/wasm-micro-runtime/releases/download/WAMR-{v}/wamrc-{v}-x86_64-windows-2022.zip",
        v = WAMRC_VERSION
    );
    install_into(
        &dir,
        WAMRC_ZIP_SHA256,
        cancel,
        on_line,
        progress,
        &mut |progress| http_fetch(&url, cancel, progress),
    )
}

/// `locate`, and `install` when nothing is found.
pub fn ensure(
    manual: Option<&Path>,
    cancel: &AtomicBool,
    on_line: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, WamrcError> {
    match locate(manual) {
        Some(p) => Ok(p),
        None => install(cancel, on_line, progress),
    }
}

/// Reads the body in pieces so progress is shown and cancel is honoured during ~20 MB.
fn http_fetch(
    url: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Vec<u8>, WamrcError> {
    let bad = |e: &dyn std::fmt::Display| WamrcError::Download(e.to_string());
    let response = ureq::get(url).call().map_err(|e| bad(&e))?;
    let total: u64 = response
        .header("Content-Length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut reader = response.into_reader();
    let mut data = Vec::with_capacity(total as usize);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(WamrcError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| bad(&e))?;
        if n == 0 {
            return Ok(data);
        }
        data.extend_from_slice(&buf[..n]);
        progress(data.len() as u64, total);
    }
}

/// Source of the archive bytes; gets the progress callback.
type Fetch<'a> = dyn FnMut(&mut dyn FnMut(u64, u64)) -> Result<Vec<u8>, WamrcError> + 'a;

/// Network-free core of `install`: `fetch` returns the archive bytes.
fn install_into(
    dir: &Path,
    expected_sha256: &str,
    cancel: &AtomicBool,
    on_line: &mut dyn FnMut(&str),
    progress: &mut dyn FnMut(u64, u64),
    fetch: &mut Fetch,
) -> Result<PathBuf, WamrcError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(WamrcError::Cancelled);
    }
    on_line(&format!("Скачиваю wamrc {WAMRC_VERSION}..."));
    let data = fetch(progress)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(WamrcError::Cancelled);
    }
    if hex(&Sha256::digest(&data)) != expected_sha256 {
        return Err(WamrcError::Checksum);
    }
    let bad = |e: &dyn std::fmt::Display| WamrcError::BadArchive(e.to_string());
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| bad(&e))?;
    // The exe may sit in a subfolder of the archive.
    let index = (0..zip.len())
        .find(|&i| {
            zip.by_index(i)
                .is_ok_and(|f| f.is_file() && f.name().rsplit('/').next() == Some(EXE_NAME))
        })
        .ok_or_else(|| bad(&"в архиве нет wamrc.exe"))?;
    let mut entry = zip.by_index(index).map_err(|e| bad(&e))?;
    std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    let target = dir.join(EXE_NAME);
    let part = dir.join(format!("{EXE_NAME}.part"));
    let written = (|| {
        let mut out = std::fs::File::create(&part).map_err(io_err(&part))?;
        std::io::copy(&mut entry, &mut out).map_err(io_err(&part))?;
        out.flush().map_err(io_err(&part))
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, &target).map_err(io_err(&target))?;
    on_line("wamrc установлен");
    Ok(target)
}

pub(crate) fn cache_key(wasm: &[u8], target: AotTarget) -> String {
    let mut hasher = Sha256::new();
    hasher.update(WAMRC_VERSION.as_bytes());
    // NUL separators so no flag/wasm boundary can alias another input.
    hasher.update([0]);
    for arg in target.args() {
        hasher.update(arg.as_bytes());
        hasher.update([0]);
    }
    hasher.update(wasm);
    hex(&hasher.finalize())[..16].to_string()
}

/// The image of `wasm` already built into `cache`, if any; needs no compiler.
pub fn cached(wasm: &[u8], target: AotTarget, cache: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(cache.join(format!("{}.aot", cache_key(wasm, target)))).ok()?;
    bytes.starts_with(b"\0aot").then_some(bytes)
}

/// Compiles `wasm` for `target`. `cache` is a folder (created if missing) where the image is kept as
/// `<first 16 hex chars of sha256(version, target args, wasm)>.aot`; a hit is returned without
/// running wamrc.
pub fn compile(
    wamrc: &Path,
    wasm: &[u8],
    target: AotTarget,
    cache: &Path,
) -> Result<Vec<u8>, WamrcError> {
    std::fs::create_dir_all(cache).map_err(io_err(cache))?;
    if let Some(bytes) = cached(wasm, target, cache) {
        return Ok(bytes);
    }
    let key = cache_key(wasm, target);
    let aot = cache.join(format!("{key}.aot"));

    let pid = std::process::id();
    let wasm_path = cache.join(format!("{key}.{pid}.wasm"));
    let out_path = cache.join(format!("{key}.{pid}.out"));
    std::fs::write(&wasm_path, wasm).map_err(io_err(&wasm_path))?;
    let result = run_wamrc(wamrc, target, &wasm_path, &out_path);
    let _ = std::fs::remove_file(&wasm_path);
    let _ = std::fs::remove_file(&out_path);
    let image = result?;

    let part = cache.join(format!("{key}.aot.part"));
    std::fs::write(&part, &image).map_err(io_err(&part))?;
    std::fs::rename(&part, &aot).map_err(io_err(&aot))?;
    Ok(image)
}

fn run_wamrc(
    wamrc: &Path,
    target: AotTarget,
    wasm_path: &Path,
    out_path: &Path,
) -> Result<Vec<u8>, WamrcError> {
    let mut cmd = std::process::Command::new(wamrc);
    cmd.args(target.args())
        .arg("-o")
        .arg(out_path)
        .arg(wasm_path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up from the GUI editor.
        cmd.creation_flags(0x0800_0000);
    }
    let output = cmd.output().map_err(|source| WamrcError::Spawn {
        path: wamrc.to_path_buf(),
        source,
    })?;
    if !output.status.success() {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        let text = text.trim();
        return Err(WamrcError::Failed(if text.is_empty() {
            format!("код выхода {}", output.status)
        } else {
            text.to_string()
        }));
    }
    let image = std::fs::read(out_path).map_err(io_err(out_path))?;
    if !image.starts_with(b"\0aot") {
        return Err(WamrcError::Failed(
            "результат не похож на AOT-образ (нет заголовка \\0aot)".to_string(),
        ));
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("crab-wamrc-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn make_zip(entry: &str, body: &[u8]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file(entry, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(body).unwrap();
        w.finish().unwrap().into_inner()
    }

    fn sha(data: &[u8]) -> String {
        hex(&Sha256::digest(data))
    }

    fn touch(p: &Path) {
        std::fs::write(p, b"x").unwrap();
    }

    #[test]
    fn locate_order() {
        let t = TempDir::new("locate");
        let manual = t.0.join("manual.exe");
        let env = t.0.join("env.exe");
        let cache = t.0.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let cached = cache.join(EXE_NAME);
        let missing = t.0.join("missing.exe");
        assert_eq!(locate_in(None, None, Some(&cache)), None);
        touch(&cached);
        assert_eq!(locate_in(None, None, Some(&cache)), Some(cached.clone()));
        touch(&env);
        assert_eq!(locate_in(None, Some(&env), Some(&cache)), Some(env.clone()));
        touch(&manual);
        assert_eq!(
            locate_in(Some(&manual), Some(&env), Some(&cache)),
            Some(manual.clone())
        );
        // A manual path that does not exist is skipped.
        assert_eq!(
            locate_in(Some(&missing), Some(&env), Some(&cache)),
            Some(env)
        );
        assert_eq!(locate_in(Some(&missing), None, Some(&cache)), Some(cached));
    }

    #[test]
    fn install_from_fake_fetch() {
        let t = TempDir::new("install");
        let dir = t.0.join("tools").join("wamrc");
        let zip = make_zip("nested/wamrc.exe", b"fake exe");
        let hash = sha(&zip);
        let cancel = AtomicBool::new(false);
        let mut fetches = 0;
        let mut lines = Vec::new();
        let exe = install_into(
            &dir,
            &hash,
            &cancel,
            &mut |l| lines.push(l.to_string()),
            &mut |_, _| {},
            &mut |_| {
                fetches += 1;
                Ok(zip.clone())
            },
        )
        .unwrap();
        assert_eq!(fetches, 1);
        assert_eq!(exe, dir.join(EXE_NAME));
        assert_eq!(std::fs::read(&exe).unwrap(), b"fake exe");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![EXE_NAME.to_string()]);
        // The second call finds it without fetching.
        assert_eq!(locate_in(None, None, Some(&dir)), Some(exe));
        assert_eq!(fetches, 1);
    }

    #[test]
    fn checksum_mismatch_leaves_nothing() {
        let t = TempDir::new("sum");
        let dir = t.0.join("tools");
        let zip = make_zip("wamrc.exe", b"fake exe");
        let cancel = AtomicBool::new(false);
        let err = install_into(
            &dir,
            &"0".repeat(64),
            &cancel,
            &mut |_| {},
            &mut |_, _| {},
            &mut |_| Ok(zip.clone()),
        )
        .unwrap_err();
        assert!(matches!(err, WamrcError::Checksum));
        assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none());
    }

    #[test]
    fn fetch_failure_mentions_manual_way() {
        let t = TempDir::new("net");
        let cancel = AtomicBool::new(false);
        let err = install_into(
            &t.0.join("tools"),
            "00",
            &cancel,
            &mut |_| {},
            &mut |_, _| {},
            &mut |_| Err(WamrcError::Download("нет сети".to_string())),
        )
        .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("интернет"), "{text}");
        assert!(text.contains("CRAB_WAMRC"), "{text}");
        assert!(text.contains("вручную"), "{text}");
    }

    #[test]
    fn cancel_flag() {
        let t = TempDir::new("cancel");
        let cancel = AtomicBool::new(true);
        let mut fetched = false;
        let err = install_into(
            &t.0.join("tools"),
            "00",
            &cancel,
            &mut |_| {},
            &mut |_, _| {},
            &mut |_| {
                fetched = true;
                Ok(Vec::new())
            },
        )
        .unwrap_err();
        assert!(matches!(err, WamrcError::Cancelled));
        assert!(!fetched);
    }

    fn cache_names(cache: &Path) -> Vec<String> {
        std::fs::read_dir(cache)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn compile_cache_hit_and_key() {
        let t = TempDir::new("cache");
        let cache = t.0.join("aot");
        let nowhere = t.0.join("no-such-wamrc.exe");
        let wasm = b"\0asm\x01\0\0\0";
        // First a miss: wamrc cannot start, the folder gets created.
        let err = compile(&nowhere, wasm, AotTarget::Riscv32Imc, &cache).unwrap_err();
        assert!(matches!(err, WamrcError::Spawn { .. }), "{err}");
        assert!(cache_names(&cache).is_empty(), "temp files left behind");

        // Learn the key by planting every candidate? Simpler: compute it the same way.
        let mut h = Sha256::new();
        h.update(WAMRC_VERSION.as_bytes());
        h.update([0]);
        for a in AotTarget::Riscv32Imc.args() {
            h.update(a.as_bytes());
            h.update([0]);
        }
        h.update(wasm);
        let key = &hex(&h.finalize())[..16];
        let image = b"\0aot cached image".to_vec();
        std::fs::write(cache.join(format!("{key}.aot")), &image).unwrap();
        let got = compile(&nowhere, wasm, AotTarget::Riscv32Imc, &cache).unwrap();
        assert_eq!(got, image);

        // Different wasm: different key, so a miss again.
        let other = b"\0asm\x01\0\0\0\x00\x01\x00";
        let err = compile(&nowhere, other, AotTarget::Riscv32Imc, &cache).unwrap_err();
        assert!(matches!(err, WamrcError::Spawn { .. }), "{err}");
    }

    #[test]
    fn real_compile() {
        let exe = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../firmware/master-rs/.deps/wamrc/wamrc.exe");
        if !exe.is_file() {
            return;
        }
        let t = TempDir::new("real");
        let cache = t.0.join("aot");
        let wasm = b"\0asm\x01\0\0\0";
        let first = compile(&exe, wasm, AotTarget::Riscv32Imc, &cache).unwrap();
        assert!(first.starts_with(b"\0aot"));
        // The second call must not need the compiler at all.
        let second = compile(&t.0.join("gone.exe"), wasm, AotTarget::Riscv32Imc, &cache).unwrap();
        assert_eq!(first, second);
    }
}
