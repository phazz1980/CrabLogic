//! Entry point of the Rust master. The board code is in `board/`; everything that does not
//! need ESP-IDF lives in the library and is tested on the desktop.

// The answers of the master go out line by line through `board::usb::write_line` (on the classic
// ESP32 one UART driver write per line: the stdout of ESP-IDF sends each byte apart and stalls
// the sheets for tens of milliseconds per answer). Defined before `board`, so it replaces the
// std macro there.
#[cfg(target_os = "espidf")]
macro_rules! println {
    () => {
        $crate::board::usb::write_line("")
    };
    ($($arg:tt)*) => {
        $crate::board::usb::write_line(&format!($($arg)*))
    };
}

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
