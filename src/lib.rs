#![deny(unsafe_op_in_unsafe_fn, unreachable_pub)]

use std::{
    env,
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
mod observer;
mod search;
mod source;
mod tui;

use app::App;
use cli::{Command, Input, OpenCommand};
pub use cli::{HELP, USAGE, VERSION_OUTPUT};
pub use document::MAX_FILE_BYTES;
pub use error::{
    ExternalSignal, InvocationError, LayoutError, LoadError, LogError, RunOutcome, SearchError,
    TutError,
};
use observer::{InputKind, Observer, SessionOutcome};

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
        Command::Open(command) => run_open(command),
    }
}

fn run_open(command: OpenCommand) -> Result<RunResult, TutError> {
    let log_file = command.log_file.or_else(|| {
        env::var_os("TUT_LOG_FILE")
            .filter(|value| !value.is_empty())
            .map(Into::into)
    });
    let mut observer = Observer::new(log_file);

    match command.input {
        Input::Path(path) => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                return Err(TutError::NotATerminal);
            }

            let handlers = tui::install_signal_handlers()?;
            let document = document::load(path)?;
            run_document(document, handlers, &mut observer, InputKind::Path)
        }
        Input::StandardInput => {
            let stdin_is_terminal = io::stdin().is_terminal();
            if !io::stdout().is_terminal() {
                return Err(TutError::NotATerminal);
            }

            if stdin_is_terminal {
                let document = document::load_standard_input(&mut io::stdin().lock())?;
                let handlers = tui::install_signal_handlers()?;
                run_document(document, handlers, &mut observer, InputKind::StandardInput)
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
                    run_document(document, handlers, &mut observer, InputKind::StandardInput)
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
    observer: &mut Observer,
    input: InputKind,
) -> Result<RunResult, TutError> {
    if let Some(signal) = handlers.state().received() {
        return Ok(RunResult::Completed(RunOutcome::Signal(signal)));
    }
    let start = observer
        .start(input, document.content_len(), document.file_identity())
        .map_err(TutError::from);
    if let Some(signal) = handlers.state().received() {
        let logging = match start {
            Ok(()) => observer
                .finish(SessionOutcome::Signal(signal))
                .map_err(TutError::from),
            Err(logging) => Err(logging),
        };
        return combine_run_and_log(
            Ok(RunResult::Completed(RunOutcome::Signal(signal))),
            logging,
        );
    }
    start?;
    let result = if let Some(signal) = handlers.state().received() {
        Ok(RunResult::Completed(RunOutcome::Signal(signal)))
    } else {
        let mut app = App::new(document);
        if let Some(signal) = handlers.state().received() {
            Ok(RunResult::Completed(RunOutcome::Signal(signal)))
        } else {
            tui::run(&mut app, handlers.state(), observer).map(RunResult::Completed)
        }
    };
    let result = promote_run_signal(result, handlers.state());
    let outcome = match &result {
        Ok(RunResult::Completed(RunOutcome::Normal)) => SessionOutcome::Normal,
        Ok(RunResult::Completed(RunOutcome::Signal(signal))) => SessionOutcome::Signal(*signal),
        Ok(RunResult::Help | RunResult::Version) | Err(_) => SessionOutcome::Error,
    };
    let logging = observer.finish(outcome).map_err(TutError::from);
    let result = promote_run_signal(result, handlers.state());
    combine_run_and_log(result, logging)
}

fn promote_run_signal(
    result: Result<RunResult, TutError>,
    signals: &tui::SignalState,
) -> Result<RunResult, TutError> {
    match (result, signals.received()) {
        (Ok(RunResult::Completed(RunOutcome::Normal)), Some(signal)) => {
            Ok(RunResult::Completed(RunOutcome::Signal(signal)))
        }
        (result, _) => result,
    }
}

fn combine_run_and_log(
    result: Result<RunResult, TutError>,
    logging: Result<(), TutError>,
) -> Result<RunResult, TutError> {
    match (result, logging) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(RunResult::Completed(RunOutcome::Signal(signal))), Err(logging)) => {
            Err(TutError::SignalAndLog {
                signal,
                logging: Box::new(logging),
            })
        }
        (Ok(_), Err(logging)) => Err(logging),
        (Err(primary), Err(logging)) => Err(TutError::PrimaryAndLog {
            primary: Box::new(primary),
            logging: Box::new(logging),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{io, path::PathBuf};

    use super::*;

    fn logging_failure() -> TutError {
        LogError::Write {
            path: PathBuf::from("session.log"),
            source: io::Error::other("write failed"),
        }
        .into()
    }

    #[test]
    fn signal_and_logging_failures_preserve_both_outcomes() {
        let combined = combine_run_and_log(
            Ok(RunResult::Completed(RunOutcome::Signal(
                ExternalSignal::Terminate,
            ))),
            Err(logging_failure()),
        )
        .unwrap_err();

        assert!(matches!(
            combined,
            TutError::SignalAndLog {
                signal: ExternalSignal::Terminate,
                ..
            }
        ));
        assert_eq!(
            combined.message(),
            "interrupted by SIGTERM; session log failed: cannot write session log 'session.log': write failed"
        );
    }
}
