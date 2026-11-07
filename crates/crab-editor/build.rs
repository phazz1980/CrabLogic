// Embeds the program icon into the exe (Explorer, taskbar, icon of `.crab` files).
fn main() {
    // Build time (local) for the status bar, so it is clear which build is open. Cargo reruns
    // the script when anything under `crates/`, `library/` or `help/` changes.
    for dir in ["..", "../../library", "../../help"] {
        println!("cargo:rerun-if-changed={dir}");
    }
    // A published release (`build-dist.ps1 -Release` sets CRAB_RELEASE) shows no mark.
    println!("cargo:rerun-if-env-changed=CRAB_RELEASE");
    let time = if std::env::var_os("CRAB_RELEASE").is_some() {
        String::new()
    } else {
        chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
    };
    println!("cargo:rustc-env=CRAB_BUILD_TIME={time}");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.rc");
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        let _ = embed_resource::compile("assets/icon.rc", embed_resource::NONE);
    }
}
