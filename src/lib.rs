#![deny(unsafe_op_in_unsafe_fn, unreachable_pub)]

use std::{
    ffi::OsString,
    io::{self, IsTerminal},
};

mod app;
mod cli;
mod document;
mod error;
mod layout;
mod search;
mod source;
mod tui;

use app::App;
use cli::Command;
pub use cli::{HELP, USAGE, VERSION_OUTPUT};
pub use document::MAX_FILE_BYTES;
pub use error::{
    ExternalSignal, InvocationError, LayoutError, LoadError, RunOutcome, SearchError, TutError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunResult {
    Help,
    Version,
    Completed(RunOutcome),
}

pub fn run<I>(args: I) -> Result<RunResult, TutError>
where
    I: IntoIterator<Item = OsString>,
{
    match cli::parse_args(args)? {
        Command::Help => Ok(RunResult::Help),
        Command::Version => Ok(RunResult::Version),
        Command::Open(path) => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                return Err(TutError::NotATerminal);
            }

            let handlers = tui::install_signal_handlers()?;
            let document = document::load(path)?;
            if let Some(signal) = handlers.state().received() {
                return Ok(RunResult::Completed(RunOutcome::Signal(signal)));
            }
            let mut app = App::new(document);
            if let Some(signal) = handlers.state().received() {
                return Ok(RunResult::Completed(RunOutcome::Signal(signal)));
            }
            Ok(RunResult::Completed(tui::run(&mut app, handlers.state())?))
        }
    }
}
