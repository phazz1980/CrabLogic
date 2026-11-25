//! `crablogic` — command line of CrabLogic.

use std::process::ExitCode;

mod bench;
mod cli;
mod report_cmd;
mod report_decode;

pub(crate) use cli::common::{find_library, load, session_blocks_dir, EXIT_GEN_FAILED};
pub(crate) use cli::link::open_master_port;

fn main() -> ExitCode {
    let code = cli::run();
    // The project's own blocks were only needed while the command ran.
    let _ = std::fs::remove_dir_all(session_blocks_dir());
    code
}
