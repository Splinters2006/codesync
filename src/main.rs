use std::{env, process::ExitCode};

fn main() -> ExitCode {
    match codesync::run_cli(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("codesync: {error}");
            ExitCode::FAILURE
        }
    }
}
