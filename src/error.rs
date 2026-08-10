use std::{
    error::Error,
    ffi::{OsStr, OsString},
    fmt, io,
    path::PathBuf,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvocationError {
    MissingPath,
    MissingPathAfterDoubleDash,
    MissingLogFile,
    EmptyLogFile,
    UnknownOption(OsString),
    UnexpectedArgument(OsString),
}

#[derive(Debug)]
pub enum LoadError {
    Open { path: PathBuf, source: io::Error },
    NotRegular(PathBuf),
    TooLarge { path: PathBuf, limit: u64 },
    InvalidUtf8 { path: PathBuf, offset: u64 },
    StandardInputTooLarge { limit: u64 },
    InvalidStandardInputUtf8 { offset: u64 },
    ReadStandardInput { source: io::Error },
    BufferStandardInput { source: io::Error },
    Allocation(&'static str),
    Read { path: PathBuf, source: io::Error },
}

#[derive(Debug)]
pub enum LogError {
    Open { path: PathBuf, source: io::Error },
    Inspect { path: PathBuf, source: io::Error },
    NotRegular(PathBuf),
    InsecurePermissions(PathBuf),
    StandardStream,
    InputConflict(PathBuf),
    Write { path: PathBuf, source: io::Error },
    EventTooLarge(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    DocumentMismatch,
    NonIncreasingRowStart {
        previous: u64,
        next: u64,
    },
    SourceRangeMismatch {
        expected_start: u64,
        expected_end: u64,
        actual_start: u64,
        actual_end: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchError {
    Allocation,
    CoordinateOverflow,
    NonIncreasingCursor { at: u64 },
    QueryTooLong { limit: usize },
    SourceMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalSignal {
    Hangup,
    Interrupt,
    Quit,
    Terminate,
}

impl ExternalSignal {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Hangup => "SIGHUP",
            Self::Interrupt => "SIGINT",
            Self::Quit => "SIGQUIT",
            Self::Terminate => "SIGTERM",
        }
    }

    #[must_use]
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Hangup => 129,
            Self::Interrupt => 130,
            Self::Quit => 131,
            Self::Terminate => 143,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Normal,
    Signal(ExternalSignal),
}

#[derive(Debug)]
pub enum TutError {
    Invocation(InvocationError),
    Load(LoadError),
    NotATerminal,
    Layout(LayoutError),
    Search(SearchError),
    Log(LogError),
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Allocation(&'static str),
    PrimaryAndRestoration {
        primary: Box<Self>,
        restoration: Box<Self>,
    },
    SignalAndRestoration {
        signal: ExternalSignal,
        restoration: Box<Self>,
    },
    PrimaryAndLog {
        primary: Box<Self>,
        logging: Box<Self>,
    },
    SignalAndLog {
        signal: ExternalSignal,
        logging: Box<Self>,
    },
}

impl TutError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Invocation(_) => 2,
            _ => 1,
        }
    }

    #[must_use]
    pub const fn show_usage(&self) -> bool {
        matches!(self, Self::Invocation(_))
    }

    #[must_use]
    pub fn message(&self) -> String {
        let message = match self {
            Self::Invocation(error) => match error {
                InvocationError::MissingPath => "missing file operand".to_owned(),
                InvocationError::MissingPathAfterDoubleDash => {
                    "missing file operand after '--'".to_owned()
                }
                InvocationError::MissingLogFile => {
                    "option '--log-file' requires an argument".to_owned()
                }
                InvocationError::EmptyLogFile => {
                    "option '--log-file' does not allow an empty argument".to_owned()
                }
                InvocationError::UnknownOption(option) => {
                    format!("unrecognized option '{}'", sanitize_os(option))
                }
                InvocationError::UnexpectedArgument(argument) => {
                    format!("extra operand '{}'", sanitize_os(argument))
                }
            },
            Self::Load(error) => load_message(error),
            Self::NotATerminal => {
                "interactive reading requires terminal input and output".to_owned()
            }
            Self::Layout(LayoutError::DocumentMismatch) => {
                "layout state belongs to another document".to_owned()
            }
            Self::Layout(LayoutError::NonIncreasingRowStart { previous, next }) => {
                format!("visual-row starts are not strictly increasing: {previous} then {next}")
            }
            Self::Layout(LayoutError::SourceRangeMismatch {
                expected_start,
                expected_end,
                actual_start,
                actual_end,
            }) => format!(
                "layout source range {expected_start}..{expected_end} does not match {actual_start}..{actual_end}"
            ),
            Self::Search(SearchError::Allocation) => {
                "could not allocate search working memory".to_owned()
            }
            Self::Search(SearchError::CoordinateOverflow) => {
                "search source coordinates overflowed".to_owned()
            }
            Self::Search(SearchError::NonIncreasingCursor { at }) => {
                format!("search did not advance from byte {at}")
            }
            Self::Search(SearchError::QueryTooLong { limit }) => {
                format!("search query exceeds the {limit}-byte limit")
            }
            Self::Search(SearchError::SourceMismatch) => {
                "search state belongs to another document".to_owned()
            }
            Self::Log(error) => log_message(error),
            Self::Io { operation, source } => format!("failed to {operation}: {source}"),
            Self::Allocation(context) => format!("could not allocate {context}"),
            Self::PrimaryAndRestoration {
                primary,
                restoration,
            } => format!(
                "{}; terminal restoration failed: {}",
                primary.message(),
                restoration.message()
            ),
            Self::SignalAndRestoration {
                signal,
                restoration,
            } => format!(
                "interrupted by {}; terminal restoration failed: {}",
                signal.name(),
                restoration.message()
            ),
            Self::PrimaryAndLog { primary, logging } => {
                format!(
                    "{}; session log failed: {}",
                    primary.message(),
                    logging.message()
                )
            }
            Self::SignalAndLog { signal, logging } => format!(
                "interrupted by {}; session log failed: {}",
                signal.name(),
                logging.message()
            ),
        };
        sanitize_text(&message)
    }
}

fn log_message(error: &LogError) -> String {
    match error {
        LogError::Open { path, source } => format!(
            "cannot open session log '{}': {source}",
            sanitize_os(path.as_os_str())
        ),
        LogError::Inspect { path, source } => format!(
            "cannot inspect session log '{}': {source}",
            sanitize_os(path.as_os_str())
        ),
        LogError::NotRegular(path) => format!(
            "session log is not a regular file: '{}'",
            sanitize_os(path.as_os_str())
        ),
        LogError::InsecurePermissions(path) => format!(
            "session log is accessible by group or other users: '{}'",
            sanitize_os(path.as_os_str())
        ),
        LogError::StandardStream => "session log must not use a standard stream".to_owned(),
        LogError::InputConflict(path) => format!(
            "session log is the input document: '{}'",
            sanitize_os(path.as_os_str())
        ),
        LogError::Write { path, source } => format!(
            "cannot write session log '{}': {source}",
            sanitize_os(path.as_os_str())
        ),
        LogError::EventTooLarge(path) => format!(
            "session log event exceeds its fixed buffer: '{}'",
            sanitize_os(path.as_os_str())
        ),
    }
}

fn load_message(error: &LoadError) -> String {
    match error {
        LoadError::Open { path, source } => {
            format!("cannot open '{}': {source}", sanitize_os(path.as_os_str()))
        }
        LoadError::NotRegular(path) => {
            format!("not a regular file: '{}'", sanitize_os(path.as_os_str()))
        }
        LoadError::TooLarge { path, limit } => format!(
            "file exceeds the {limit}-byte limit: '{}'",
            sanitize_os(path.as_os_str())
        ),
        LoadError::InvalidUtf8 { path, offset } => format!(
            "invalid UTF-8 at byte {offset}: '{}'",
            sanitize_os(path.as_os_str())
        ),
        LoadError::StandardInputTooLarge { limit } => {
            format!("standard input exceeds the {limit}-byte limit")
        }
        LoadError::InvalidStandardInputUtf8 { offset } => {
            format!("invalid UTF-8 in standard input at byte {offset}")
        }
        LoadError::ReadStandardInput { source } => {
            format!("cannot read standard input: {source}")
        }
        LoadError::BufferStandardInput { source } => {
            format!("cannot buffer standard input: {source}")
        }
        LoadError::Allocation(context) => format!("could not allocate {context}"),
        LoadError::Read { path, source } => {
            format!("cannot read '{}': {source}", sanitize_os(path.as_os_str()))
        }
    }
}

pub(super) fn sanitize_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '\u{0000}'..='\u{001f}' | '\u{007f}' => {
                use fmt::Write as _;
                let _ = write!(output, "\\x{:02x}", character as u32);
            }
            character if is_terminal_control(character) => {
                use fmt::Write as _;
                let _ = write!(output, "\\u{{{:04x}}}", character as u32);
            }
            _ => output.push(character),
        }
    }
    output
}

pub(super) const fn is_terminal_control(character: char) -> bool {
    matches!(
        character,
        '\u{0000}'..='\u{001f}'
            | '\u{007f}'..='\u{009f}'
            | '\u{061c}'
            | '\u{200e}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

pub(super) fn sanitize_os(input: &OsStr) -> String {
    sanitize_text(&input.to_string_lossy())
}

impl fmt::Display for TutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl Error for TutError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Load(
                LoadError::Open { source, .. }
                | LoadError::Read { source, .. }
                | LoadError::ReadStandardInput { source }
                | LoadError::BufferStandardInput { source },
            )
            | Self::Log(
                LogError::Open { source, .. }
                | LogError::Inspect { source, .. }
                | LogError::Write { source, .. },
            )
            | Self::Io { source, .. } => Some(source),
            Self::PrimaryAndRestoration { primary, .. } => Some(primary.as_ref()),
            Self::SignalAndRestoration { restoration, .. } => Some(restoration.as_ref()),
            Self::PrimaryAndLog { primary, .. } => Some(primary.as_ref()),
            Self::SignalAndLog { logging, .. } => Some(logging.as_ref()),
            _ => None,
        }
    }
}

impl From<InvocationError> for TutError {
    fn from(error: InvocationError) -> Self {
        Self::Invocation(error)
    }
}

impl From<LoadError> for TutError {
    fn from(error: LoadError) -> Self {
        Self::Load(error)
    }
}

impl From<LayoutError> for TutError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

impl From<SearchError> for TutError {
    fn from(error: SearchError) -> Self {
        Self::Search(error)
    }
}

impl From<LogError> for TutError {
    fn from(error: LogError) -> Self {
        Self::Log(error)
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, io, path::PathBuf};

    use super::*;

    #[test]
    fn sanitization_is_single_line_and_control_safe() {
        assert_eq!(
            sanitize_text("a\0\n\u{001b}\u{007f}\u{0085}\u{061c}\u{200e}\u{202e}\u{2066}z"),
            "a\\x00\\x0a\\x1b\\x7f\\u{0085}\\u{061c}\\u{200e}\\u{202e}\\u{2066}z"
        );
        assert_eq!(sanitize_os(OsStr::new("safe")), "safe");
    }

    #[test]
    fn errors_have_stable_messages_and_exit_codes() {
        let invocation = TutError::Invocation(InvocationError::MissingPath);
        assert_eq!(invocation.message(), "missing file operand");
        assert_eq!(invocation.exit_code(), 2);
        assert!(invocation.show_usage());

        let invalid = TutError::Load(LoadError::InvalidUtf8 {
            path: PathBuf::from("bad\n.txt"),
            offset: 2,
        });
        assert_eq!(invalid.message(), "invalid UTF-8 at byte 2: 'bad\\x0a.txt'");
        assert_eq!(invalid.exit_code(), 1);

        let stdin = TutError::Load(LoadError::InvalidStandardInputUtf8 { offset: 16 });
        assert_eq!(
            stdin.message(),
            "invalid UTF-8 in standard input at byte 16"
        );

        let log = TutError::Log(LogError::InputConflict(PathBuf::from("bad\n.log")));
        assert_eq!(
            log.message(),
            "session log is the input document: 'bad\\x0a.log'"
        );
    }

    #[test]
    fn primary_and_restoration_failures_are_both_retained() {
        let error = TutError::PrimaryAndRestoration {
            primary: Box::new(TutError::Io {
                operation: "draw terminal frame",
                source: io::Error::other("draw failed"),
            }),
            restoration: Box::new(TutError::Io {
                operation: "show cursor",
                source: io::Error::other("restore\nfailed"),
            }),
        };
        assert_eq!(
            error.message(),
            "failed to draw terminal frame: draw failed; terminal restoration failed: failed to show cursor: restore\\x0afailed"
        );
        assert_eq!(
            error.source().unwrap().to_string(),
            "failed to draw terminal frame: draw failed"
        );

        let logging = TutError::PrimaryAndLog {
            primary: Box::new(error),
            logging: Box::new(TutError::Log(LogError::Write {
                path: PathBuf::from("session.log"),
                source: io::Error::other("disk full"),
            })),
        };
        assert!(logging.message().contains("terminal restoration failed"));
        assert!(
            logging
                .message()
                .ends_with("session log failed: cannot write session log 'session.log': disk full")
        );
    }
}
