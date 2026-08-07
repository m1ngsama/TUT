use std::{
    env,
    io::{self, Write},
    process::ExitCode,
};

use tut::{HELP, RunOutcome, RunResult, USAGE, VERSION_OUTPUT};

fn main() -> ExitCode {
    match tut::run(env::args_os().skip(1)) {
        Ok(RunResult::Help) => write_stdout(HELP),
        Ok(RunResult::Version) => write_stdout(VERSION_OUTPUT),
        Ok(RunResult::Completed(RunOutcome::Normal)) => ExitCode::SUCCESS,
        Ok(RunResult::Completed(RunOutcome::Signal(signal))) => {
            let _ = writeln!(io::stderr().lock(), "tut: interrupted by {}", signal.name());
            ExitCode::from(signal.exit_code())
        }
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "tut: {}", error.message());
            if error.show_usage() {
                let _ = writeln!(io::stderr().lock(), "{USAGE}");
                let _ = writeln!(
                    io::stderr().lock(),
                    "Try 'tut --help' for more information."
                );
            }
            ExitCode::from(error.exit_code())
        }
    }
}

fn write_stdout(text: &str) -> ExitCode {
    match io::stdout().lock().write_all(text.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(1),
    }
}
