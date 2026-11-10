//! Entry point of the Rust master. The board code is in `board/`; everything that does not
//! need ESP-IDF lives in the library and is tested on the desktop.

#[cfg(target_os = "espidf")]
mod board;

#[cfg(target_os = "espidf")]
fn main() {
    board::main();
}

#[cfg(not(target_os = "espidf"))]
fn main() {
    // Keep `cargo run` useful on the host without pretending to run WAMR.
    println!("crab-master-rs: соберите под ESP-IDF для запуска мастера");
}
