use std::{ffi::OsString, path::PathBuf};

use crate::error::InvocationError;

pub const USAGE: &str = "Usage: tut [OPTION]... FILE";
pub const HELP: &str = "\
Usage: tut [OPTION]... FILE
Read a local UTF-8 text file in the terminal.

  -h, --help     display this help and exit
  -V, --version  output version information and exit

Use -- before a FILE whose name begins with '-'.

Report bugs to: https://github.com/m1ngsama/TUT/issues
TUT home page: https://github.com/m1ngsama/TUT
";
pub const VERSION_OUTPUT: &str = concat!(
    "tut (TUT) ",
    env!("CARGO_PKG_VERSION"),
    "\nCopyright (C) 2024 m1ngsama\n",
    "License MIT: <https://opensource.org/license/mit>.\n",
    "This is free software: you are free to change and redistribute it.\n",
    "There is NO WARRANTY, to the extent permitted by law.\n",
    "Written by m1ngsama.\n",
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Command {
    Help,
    Version,
    Open(PathBuf),
}

pub(super) fn parse_args<I>(args: I) -> Result<Command, InvocationError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let first = args.next().ok_or(InvocationError::MissingPath)?;

    if first == "-h" || first == "--help" {
        return finish_flag(args, Command::Help);
    }
    if first == "-V" || first == "--version" {
        return finish_flag(args, Command::Version);
    }
    if first == "--" {
        let path = args
            .next()
            .ok_or(InvocationError::MissingPathAfterDoubleDash)?;
        return finish_path(args, path);
    }
    if first.to_string_lossy().starts_with('-') {
        return Err(InvocationError::UnknownOption(first));
    }

    finish_path(args, first)
}

fn finish_flag(
    mut args: impl Iterator<Item = OsString>,
    command: Command,
) -> Result<Command, InvocationError> {
    match args.next() {
        None => Ok(command),
        Some(extra) => Err(InvocationError::UnexpectedArgument(extra)),
    }
}

fn finish_path(
    mut args: impl Iterator<Item = OsString>,
    path: OsString,
) -> Result<Command, InvocationError> {
    match args.next() {
        None => Ok(Command::Open(PathBuf::from(path))),
        Some(extra) => Err(InvocationError::UnexpectedArgument(extra)),
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::*;

    fn parse(values: &[&str]) -> Result<Command, InvocationError> {
        parse_args(values.iter().map(OsString::from))
    }

    #[test]
    fn parses_supported_invocations() {
        assert_eq!(parse(&["-h"]), Ok(Command::Help));
        assert_eq!(parse(&["--help"]), Ok(Command::Help));
        assert_eq!(parse(&["-V"]), Ok(Command::Version));
        assert_eq!(parse(&["--version"]), Ok(Command::Version));
        assert_eq!(
            parse(&["book.txt"]),
            Ok(Command::Open(PathBuf::from("book.txt")))
        );
        assert_eq!(
            parse(&["--", "-book.txt"]),
            Ok(Command::Open(PathBuf::from("-book.txt")))
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
        assert_eq!(
            parse(&["a", "b"]),
            Err(InvocationError::UnexpectedArgument(OsString::from("b")))
        );
    }

    #[test]
    fn help_and_version_follow_gnu_output_conventions() {
        assert!(HELP.starts_with("Usage: tut "));
        assert!(HELP.ends_with('\n'));
        assert!(VERSION_OUTPUT.starts_with(concat!("tut (TUT) ", env!("CARGO_PKG_VERSION"), "\n")));
        assert!(VERSION_OUTPUT.contains("License MIT"));
        assert!(VERSION_OUTPUT.ends_with('\n'));
    }
}
