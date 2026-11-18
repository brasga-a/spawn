//! `spawn-engine` executable entry point.

use std::process::ExitCode;

fn main() -> ExitCode {
    match spawn_engine::run() {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
