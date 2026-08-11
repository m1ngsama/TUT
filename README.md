# TUT

[![CI](https://github.com/m1ngsama/TUT/actions/workflows/ci.yml/badge.svg)](https://github.com/m1ngsama/TUT/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

TUT is a focused plain-text reader for Unix terminals.

It reads one finite UTF-8 document, presents it safely, and gets out of the
way. TUT does not edit text, interpret document control sequences, or try to
replace the programs that fetch, filter, decompress, and convert content. Those
programs compose with TUT through standard input.

TUT is under active pre-1.0 development. The current product is deliberately
narrow: a reliable local reader, not yet a complete replacement for `less`.

## What it provides

- UTF-8 and Unicode-grapheme-aware display with automatic soft wrapping.
- Vim- and pager-style line, page, half-page, and document navigation.
- Exact, case-sensitive literal search with highlighting and match navigation.
- Bounded numeric prefixes such as `12j`, `3 Space`, and `5n`.
- Responsive status information, exact `TOP`/`END`/`ALL` markers, and F1 help.
- Incremental layout, indexing, rendering, and search work so input and signals
  remain observable during expensive operations.
- Explicit handling of terminal resize, suspension, continuation, EOF, HUP,
  interrupts, and terminal restoration.
- Safe projection of control and bidirectional-formatting characters instead
  of executing or invisibly applying them.
- Optional local session metrics that exclude document text, queries, paths,
  and individual keys.

TUT follows the Unix idea of doing one job well. Its product and engineering
contract is described in [the project principles](docs/PROJECT.md).

## Install from source

TUT currently targets Linux and macOS. Building requires Git and Rust 1.88.0
or newer.

```sh
git clone https://github.com/m1ngsama/TUT.git
cd TUT
cargo install --path . --locked
```

The repository also provides GNU-style Make targets:

```sh
make
make install prefix="$HOME/.local"
```

The Make path installs both the executable and [tut(1)](docs/tut.1); the Cargo
command installs only the executable. `prefix`, `exec_prefix`, `bindir`,
`mandir`, and `DESTDIR` may be overridden. Use the same prefix with
`make uninstall` to remove the installed files.

## Usage

Read a regular file:

```sh
tut notes.txt
```

Read finite output from another Unix program:

```sh
rg --color=never 'pattern' . | tut -
gzip -cd archive.txt.gz | tut -
```

When content comes from a pipe, TUT reads commands from the controlling
terminal through `/dev/tty`. Standard input is validated and buffered to a
private snapshot through EOF before the interface opens; TUT is not a live
stream follower.

<!-- BEGIN TUT HELP -->
```text
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
```
<!-- END TUT HELP -->

Use `-` as `FILE` to read standard input. Use `--` before a filename beginning
with `-`; use `./-` to open a file literally named `-`.

`TUT_LOG_FILE` is an alternative to `--log-file`; the command-line option wins
when both are set.

## Keys

Press `F1` in TUT for a guide adapted to the current terminal size.

### Reading

| Key | Action |
| --- | --- |
| `j`, Down | Next visual line |
| `k`, Up | Previous visual line |
| Space, PageDown, `Ctrl-F` | Next page |
| `b`, PageUp, `Ctrl-B` | Previous page |
| `Ctrl-D`, `Ctrl-U` | Half page down or up |
| `g`, Home | Start of document |
| `G`, End | End of document |
| `/` | Enter search |
| `n`, `N` | Next or previous match |
| `1`-`9999` before a relative motion | Repeat the motion |
| Backspace | Edit an active numeric prefix |
| Esc | Cancel a prefix or clear the committed search |
| `F1` | Open or close help |
| `q` | Quit |
| `Ctrl-C` | Interrupt |

Counts apply to line, page, half-page, and match movement. A count starts with
`1`-`9`; subsequent digits may include `0`. A digit that would raise the count
above 9999 is rejected while the existing prefix is preserved.

### Search input

| Key | Action |
| --- | --- |
| Text | Extend the draft query; `q` is text in this mode |
| Backspace | Remove the previous grapheme |
| Up | Recall the committed query when the draft is empty |
| `Ctrl-U` | Clear the draft |
| Enter | Apply the draft; an empty draft clears committed search |
| Esc | Cancel the draft and retain committed search |
| `F1` | Open help without losing the draft |
| `Ctrl-C` | Interrupt |

Search is literal, case-sensitive, non-overlapping, and limited to 4096 UTF-8
bytes. `n` and `N` wrap around the document. Up does nothing when the draft is
nonempty, and TUT does not maintain a multi-entry search history. Help opened
from search input closes only with Esc or `F1`.

## Input contract and limits

| Property | Current behavior |
| --- | --- |
| Input | One regular file or one finite standard-input snapshot |
| Encoding | Valid UTF-8; an initial UTF-8 BOM is accepted and hidden |
| Maximum input size | 32 MiB, including standard input |
| Layout | Automatic soft wrap; no horizontal scrolling or no-wrap mode |
| Line endings | LF, CRLF, and CR are recognized |
| Tabs | Expanded to four-column tab stops |
| Control sequences | Displayed inertly; ANSI styling is not interpreted |
| Oversized graphemes | A grapheme over 1024 UTF-8 bytes is replaced safely |
| Opened file mutation | Fingerprint changes are reported as an error |
| Minimum terminal | 16 columns by 4 rows for the reader view |
| Terminal frames | Two cell buffers and heap symbols share a 64 MiB budget |
| Tested systems | Linux and macOS |

Windows is not supported. Other Unix systems may work, but are not currently
covered by the terminal and signal test matrix.

TUT does not currently follow or reload growing files, switch between multiple
documents, run regular-expression searches, or provide mouse and configuration
systems. Convert other document formats to finite UTF-8 plain text before
piping them to TUT.

## Session logging

Session logging is disabled unless `--log-file` or `TUT_LOG_FILE` is supplied.
The log is opened in append mode and contains a schema marker, aggregate source
size and runtime metrics, and the session outcome. It does not contain document
content, filenames, search queries, or keystrokes.

New logs are created with owner-only permissions. Existing logs must be regular
files with no group or other permissions, and a log may not alias the input
document or a standard stream.

## Development

The normal local checks mirror the CI gates:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
make distcheck
```

CI exercises the current toolchain and Rust 1.88.0 on Ubuntu, plus the current
toolchain on macOS. See [CONTRIBUTING.md](CONTRIBUTING.md) before proposing a
change and [CHANGELOG.md](CHANGELOG.md) for user-visible release history.

The terminal-reader product starts at `v0.0.2` and uses the `0.0.x` version
line. The older `v0.0.1`, `v2`, and date/hash tags belong to an earlier
browser-oriented project and are retained only as repository history; they do
not describe this program's compatibility or maturity.

## Reporting problems

Report reproducible bugs and focused feature proposals through
[GitHub Issues](https://github.com/m1ngsama/TUT/issues). For a vulnerability or
other security-sensitive report, follow [SECURITY.md](SECURITY.md) instead of
opening a public issue.

## License

TUT is independent free software released under the [MIT License](LICENSE). It
takes inspiration from established Unix and GNU tools but is not affiliated
with the GNU Project.
