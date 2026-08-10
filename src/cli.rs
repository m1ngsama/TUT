use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt as _, OsStringExt as _},
    path::PathBuf,
};

use crate::error::InvocationError;

pub const USAGE: &str = "Usage: tut [OPTION]... FILE";
pub const HELP: &str = "\
Usage: tut [OPTION]... FILE
Read UTF-8 text from FILE in the terminal.

  -h, --help     display this help and exit
  -V, --version  output version information and exit
      --log-file=FILE
                  append typed session events to FILE

With FILE -, read standard input.
Use -- before a FILE whose name begins with '-'.
For a file named '-', use ./-.
TUT_LOG_FILE is used when --log-file is not specified.

Report bugs to: https://github.com/m1ngsama/TUT/issues
TUT home page: https://github.com/m1ngsama/TUT
";
pub const VERSION_OUTPUT: &str = concat!(
    "tut (TUT) ",
    env!("CARGO_PKG_VERSION"),
    "\nCopyright (C) 2024-2026 m1ngsama\n",
    "License MIT: <https://opensource.org/license/mit>.\n",
    "This is free software: you are free to change and redistribute it.\n",
    "There is NO WARRANTY, to the extent permitted by law.\n",
    "Written by m1ngsama.\n",
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Command {
    Help,
    Version,
    Open(OpenCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpenCommand {
    pub(super) input: Input,
    pub(super) log_file: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Input {
    Path(PathBuf),
    StandardInput,
}

pub(super) fn parse_args<I>(args: I) -> Result<Command, InvocationError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut arguments = args.into_iter();
    let mut path = None;
    let mut log_file = None;
    let mut error = None;
    let mut options = true;
    let mut saw_double_dash = false;

    while let Some(argument) = arguments.next() {
        if options && argument == "--" {
            options = false;
            saw_double_dash = true;
            continue;
        }
        if options && (argument == "-h" || argument == "--help") {
            return Ok(Command::Help);
        }
        if options && (argument == "-V" || argument == "--version") {
            return Ok(Command::Version);
        }
        if options && argument == "--log-file" {
            match arguments.next() {
                Some(value) if !value.is_empty() => log_file = Some(PathBuf::from(value)),
                Some(_) if error.is_none() => error = Some(InvocationError::EmptyLogFile),
                _ if error.is_none() => error = Some(InvocationError::MissingLogFile),
                _ => {}
            }
            continue;
        }
        if options && let Some(value) = long_option_value(&argument, b"--log-file=") {
            if value.is_empty() {
                if error.is_none() {
                    error = Some(InvocationError::EmptyLogFile);
                }
            } else {
                log_file = Some(PathBuf::from(value));
            }
            continue;
        }
        if error.is_some() {
            continue;
        }
        if options && argument != "-" && argument.to_string_lossy().starts_with('-') {
            error = Some(InvocationError::UnknownOption(argument));
        } else if path.is_none() {
            path = Some(argument);
        } else {
            error = Some(InvocationError::UnexpectedArgument(argument));
        }
    }

    error.map_or_else(
        || match path {
            Some(path) if path == "-" => Ok(Command::Open(OpenCommand {
                input: Input::StandardInput,
                log_file,
            })),
            Some(path) => Ok(Command::Open(OpenCommand {
                input: Input::Path(PathBuf::from(path)),
                log_file,
            })),
            None if saw_double_dash => Err(InvocationError::MissingPathAfterDoubleDash),
            None => Err(InvocationError::MissingPath),
        },
        Err,
    )
}

fn long_option_value(argument: &OsStr, prefix: &[u8]) -> Option<OsString> {
    argument
        .as_bytes()
        .strip_prefix(prefix)
        .map(|value| OsString::from_vec(value.to_vec()))
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::*;

    fn parse(values: &[&str]) -> Result<Command, InvocationError> {
        parse_args(values.iter().map(OsString::from))
    }

    fn open(input: Input) -> Command {
        Command::Open(OpenCommand {
            input,
            log_file: None,
        })
    }

    #[test]
    fn parses_supported_invocations() {
        assert_eq!(parse(&["-h"]), Ok(Command::Help));
        assert_eq!(parse(&["--help"]), Ok(Command::Help));
        assert_eq!(parse(&["-V"]), Ok(Command::Version));
        assert_eq!(parse(&["--version"]), Ok(Command::Version));
        assert_eq!(
            parse(&["book.txt"]),
            Ok(open(Input::Path(PathBuf::from("book.txt"))))
        );
        assert_eq!(
            parse(&["--", "-book.txt"]),
            Ok(open(Input::Path(PathBuf::from("-book.txt"))))
        );
        assert_eq!(
            parse(&["book.txt", "--"]),
            Ok(open(Input::Path(PathBuf::from("book.txt"))))
        );
        assert_eq!(
            parse(&["--", "--help"]),
            Ok(open(Input::Path(PathBuf::from("--help"))))
        );
        assert_eq!(parse(&["-"]), Ok(open(Input::StandardInput)));
        assert_eq!(parse(&["--", "-"]), Ok(open(Input::StandardInput)));
        assert_eq!(parse(&["./-"]), Ok(open(Input::Path(PathBuf::from("./-")))));
        assert_eq!(
            parse(&["--log-file=session.log", "book.txt"]),
            Ok(Command::Open(OpenCommand {
                input: Input::Path(PathBuf::from("book.txt")),
                log_file: Some(PathBuf::from("session.log")),
            }))
        );
        assert_eq!(
            parse(&["book.txt", "--log-file", "later.log"]),
            Ok(Command::Open(OpenCommand {
                input: Input::Path(PathBuf::from("book.txt")),
                log_file: Some(PathBuf::from("later.log")),
            }))
        );
    }

    #[test]
    fn information_options_short_circuit_earlier_and_later_errors() {
        assert_eq!(parse(&["book.txt", "extra", "--help"]), Ok(Command::Help));
        assert_eq!(parse(&["--unknown", "-h"]), Ok(Command::Help));
        assert_eq!(
            parse(&["book.txt", "--unknown", "--version", "extra"]),
            Ok(Command::Version)
        );
        assert_eq!(parse(&["--version", "--help"]), Ok(Command::Version));
        assert_eq!(parse(&["--help", "--version"]), Ok(Command::Help));
        assert_eq!(
            parse(&["--log-file", "--help", "book.txt"]),
            Ok(Command::Open(OpenCommand {
                input: Input::Path(PathBuf::from("book.txt")),
                log_file: Some(PathBuf::from("--help")),
            }))
        );
        assert_eq!(parse(&["-", "--help"]), Ok(Command::Help));
        assert_eq!(
            parse(&["--", "-", "--help"]),
            Err(InvocationError::UnexpectedArgument(OsString::from(
                "--help"
            )))
        );
    }

    #[test]
    fn rejects_invalid_invocations_without_losing_os_strings() {
        assert!(matches!(parse(&[]), Err(InvocationError::MissingPath)));
        assert!(matches!(
            parse(&["--"]),
            Err(InvocationError::MissingPathAfterDoubleDash)
        ));
        assert_eq!(
            parse(&["--wat"]),
            Err(InvocationError::UnknownOption(OsString::from("--wat")))
        );
        assert!(matches!(
            parse(&["--log-file"]),
            Err(InvocationError::MissingLogFile)
        ));
        assert!(matches!(
            parse(&["--log-file=", "book.txt"]),
            Err(InvocationError::EmptyLogFile)
        ));
        assert!(matches!(
            parse(&["--log-file", "", "book.txt"]),
            Err(InvocationError::EmptyLogFile)
        ));
        assert_eq!(
            parse(&["a", "b"]),
            Err(InvocationError::UnexpectedArgument(OsString::from("b")))
        );
    }

    #[test]
    fn log_file_options_preserve_non_utf8_paths() {
        let value = OsString::from_vec(b"--log-file=log-\xff".to_vec());
        assert_eq!(
            parse_args([value, OsString::from("book.txt")]),
            Ok(Command::Open(OpenCommand {
                input: Input::Path(PathBuf::from("book.txt")),
                log_file: Some(PathBuf::from(OsString::from_vec(b"log-\xff".to_vec()))),
            }))
        );
    }

    #[test]
    fn help_and_version_follow_gnu_output_conventions() {
        assert!(HELP.starts_with("Usage: tut "));
        assert!(HELP.ends_with('\n'));
        assert!(HELP.contains("With FILE -, read standard input."));
        assert!(HELP.contains("--log-file=FILE"));
        assert!(HELP.contains("TUT_LOG_FILE"));
        assert!(HELP.contains("For a file named '-', use ./-."));
        assert!(VERSION_OUTPUT.starts_with(concat!("tut (TUT) ", env!("CARGO_PKG_VERSION"), "\n")));
        assert!(VERSION_OUTPUT.contains("Copyright (C) 2024-2026 m1ngsama"));
        assert!(VERSION_OUTPUT.contains("License MIT"));
        assert!(VERSION_OUTPUT.ends_with('\n'));
    }
}
