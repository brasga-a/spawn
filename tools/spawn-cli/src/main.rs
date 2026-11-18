//! Binary entrypoint for `spawn` (`spawn-cli`, `ADR-0014`).

#![forbid(unsafe_code)]

use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut stdout = io::stdout().lock();
    match spawn_cli::run_cli(std::env::args_os(), &mut stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let mut stderr = io::stderr().lock();
            let _ = writeln!(stderr, "{err}");
            ExitCode::from(err.exit_code())
        }
    }
}
