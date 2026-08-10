#![deny(unsafe_op_in_unsafe_fn, unreachable_pub)]

use std::{
    ffi::{OsStr, OsString},
    fs::OpenOptions,
    io::{self, IsTerminal},
    os::fd::{AsFd, OwnedFd},
    os::unix::ffi::OsStrExt,
    path::Path,
};

mod app;
mod cli;
mod document;
mod error;
mod layout;
mod line_index;
mod locator;
mod search;
mod source;
mod tui;

use app::App;
use cli::{Command, Input};
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
        Command::Open(Input::Path(path)) => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                return Err(TutError::NotATerminal);
            }

            let handlers = tui::install_signal_handlers()?;
            let document = document::load(path)?;
            run_document(document, handlers)
        }
        Command::Open(Input::StandardInput) => {
            let stdin_is_terminal = io::stdin().is_terminal();
            if !io::stdout().is_terminal() {
                return Err(TutError::NotATerminal);
            }

            if stdin_is_terminal {
                let document = document::load_standard_input(&mut io::stdin().lock())?;
                let handlers = tui::install_signal_handlers()?;
                run_document(document, handlers)
            } else {
                let original = duplicate_descriptor(&rustix::stdio::stdin())
                    .map_err(|source| LoadError::ReadStandardInput { source })?;
                let terminal = open_terminal_input().ok_or(TutError::NotATerminal)?;
                let redirect = StandardInputRedirect::attach(original, &terminal)?;
                drop(terminal);
                let result = (|| {
                    let document = {
                        let mut reader = redirect.input_reader()?;
                        document::load_standard_input(&mut reader)?
                    };
                    let handlers = tui::install_signal_handlers()?;
                    run_document(document, handlers)
                })();
                redirect.finish(result)
            }
        }
    }
}

fn open_terminal_input() -> Option<std::fs::File> {
    let name = rustix::termios::ttyname(rustix::stdio::stdout(), Vec::new()).ok()?;
    let path = Path::new(OsStr::from_bytes(name.to_bytes()));
    OpenOptions::new().read(true).write(true).open(path).ok()
}

struct StandardInputRedirect {
    original: Option<OwnedFd>,
}

impl StandardInputRedirect {
    fn attach(original: OwnedFd, terminal: &std::fs::File) -> Result<Self, TutError> {
        replace_standard_input(terminal).map_err(|source| TutError::Io {
            operation: "attach terminal input",
            source,
        })?;
        Ok(Self {
            original: Some(original),
        })
    }

    fn input_reader(&self) -> Result<std::fs::File, TutError> {
        let input = duplicate_descriptor(
            self.original
                .as_ref()
                .expect("redirected standard input retains its source"),
        )
        .map_err(|source| TutError::Io {
            operation: "duplicate standard input",
            source,
        })?;
        Ok(input.into())
    }

    fn restore(&mut self) -> Result<(), TutError> {
        let original = self
            .original
            .as_ref()
            .expect("standard-input restoration runs once");
        replace_standard_input(original).map_err(|source| TutError::Io {
            operation: "restore standard input",
            source,
        })?;
        self.original = None;
        Ok(())
    }

    fn finish(mut self, result: Result<RunResult, TutError>) -> Result<RunResult, TutError> {
        let restoration = self.restore();
        match (result, restoration) {
            (Ok(result), Ok(())) => Ok(result),
            (Err(primary), Ok(())) => Err(primary),
            (Ok(RunResult::Completed(RunOutcome::Signal(signal))), Err(restoration)) => {
                Err(TutError::SignalAndRestoration {
                    signal,
                    restoration: Box::new(restoration),
                })
            }
            (Ok(_), Err(restoration)) => Err(restoration),
            (Err(primary), Err(restoration)) => Err(TutError::PrimaryAndRestoration {
                primary: Box::new(primary),
                restoration: Box::new(restoration),
            }),
        }
    }
}

fn duplicate_descriptor(source: &impl AsFd) -> io::Result<OwnedFd> {
    loop {
        match rustix::io::fcntl_dupfd_cloexec(source, 3) {
            Ok(duplicate) => return Ok(duplicate),
            Err(error) => {
                let error = io::Error::from(error);
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
}

fn replace_standard_input(source: &impl AsFd) -> io::Result<()> {
    loop {
        match rustix::stdio::dup2_stdin(source) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let error = io::Error::from(error);
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
}

impl Drop for StandardInputRedirect {
    fn drop(&mut self) {
        if self.original.is_some() {
            let _ = self.restore();
        }
    }
}

fn run_document(
    document: document::Document,
    handlers: tui::SignalHandlers,
) -> Result<RunResult, TutError> {
    if let Some(signal) = handlers.state().received() {
        return Ok(RunResult::Completed(RunOutcome::Signal(signal)));
    }
    let mut app = App::new(document);
    if let Some(signal) = handlers.state().received() {
        return Ok(RunResult::Completed(RunOutcome::Signal(signal)));
    }
    Ok(RunResult::Completed(tui::run(&mut app, handlers.state())?))
}
