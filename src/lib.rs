#![deny(unsafe_op_in_unsafe_fn, unreachable_pub)]

use std::{
    env,
    ffi::OsString,
    fs::OpenOptions,
    io::{self, IsTerminal},
    os::fd::AsFd,
};

mod app;
mod cli;
mod document;
mod error;
mod layout;
mod line_index;
mod locator;
mod observer;
mod path_binding;
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
    let lease = tui::acquire_session()?;
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

            let mut handlers = tui::install_signal_handlers(lease)?;
            let result = match document::load(path) {
                Ok(document) => {
                    run_document(document, &mut handlers, &mut observer, InputKind::Path)
                }
                Err(error) => Err(error.into()),
            };
            finish_signal_handlers(handlers, result)
        }
        Input::StandardInput => {
            let stdin_is_terminal = io::stdin().is_terminal();
            if !io::stdout().is_terminal() {
                return Err(TutError::NotATerminal);
            }

            if !stdin_is_terminal {
                ensure_controlling_terminal()?;
            }
            let document = {
                let stdin = io::stdin();
                let mut input = stdin.lock();
                ensure_standard_input_readable(&input)?;
                document::load_standard_input(&mut input)?
            };
            let mut handlers = tui::install_signal_handlers(lease)?;
            let result = run_document(
                document,
                &mut handlers,
                &mut observer,
                InputKind::StandardInput,
            );
            finish_signal_handlers(handlers, result)
        }
    }
}

fn ensure_controlling_terminal() -> Result<(), TutError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map(drop)
        .map_err(|_| TutError::NotATerminal)
}

fn ensure_standard_input_readable(input: &impl AsFd) -> Result<(), LoadError> {
    let flags = rustix::fs::fcntl_getfl(input).map_err(|source| LoadError::ReadStandardInput {
        source: source.into(),
    })?;
    if flags & rustix::fs::OFlags::RWMODE == rustix::fs::OFlags::WRONLY {
        return Err(LoadError::ReadStandardInput {
            source: io::Error::from(rustix::io::Errno::BADF),
        });
    }
    Ok(())
}

fn run_document(
    document: document::Document,
    handlers: &mut tui::SignalHandlers,
    observer: &mut Observer,
    input: InputKind,
) -> Result<RunResult, TutError> {
    if let Some(signal) = handlers.state().received() {
        return Ok(RunResult::Completed(RunOutcome::Signal(signal)));
    }
    let start = observer
        .start(input, document.content_len(), document.input_identity())
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
            tui::run(&mut app, handlers, observer).map(RunResult::Completed)
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

fn finish_signal_handlers(
    mut handlers: tui::SignalHandlers,
    result: Result<RunResult, TutError>,
) -> Result<RunResult, TutError> {
    let result = promote_run_signal(result, handlers.state());
    let restoration = handlers.restore().map_err(|source| TutError::Io {
        operation: "restore signal handlers",
        source,
    });
    let result = promote_run_signal(result, handlers.state());
    match (result, restoration) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(RunResult::Completed(RunOutcome::Signal(signal))), Err(restoration)) => {
            Err(TutError::SignalAndSignalHandlerRestoration {
                signal,
                restoration: Box::new(restoration),
            })
        }
        (Ok(_), Err(restoration)) => Err(restoration),
        (Err(primary), Err(restoration)) => Err(TutError::PrimaryAndSignalHandlerRestoration {
            primary: Box::new(primary),
            restoration: Box::new(restoration),
        }),
    }
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
