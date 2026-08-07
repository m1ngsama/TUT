# TUT

TUT 0.0.1 is a local plain-text reader for macOS and Linux terminals. It reads one UTF-8 file, wraps text by terminal cell width, and provides navigation and literal search without modifying the source.

## Requirements

- Rust 1.88.0 or later
- Cargo
- GNU Make for the conventional build interface
- A UTF-8 terminal with separate terminal stdin and stdout

## Build and install

```sh
make
make check
make install prefix=/usr/local
```

Package builders can stage an installation without changing the live system:

```sh
make install DESTDIR="$package_root" prefix=/usr
make installcheck DESTDIR="$package_root" prefix=/usr
```

Cargo remains available as the lower-level interface:

```sh
cargo build --release --locked
cargo test --all-targets --locked
```

## Usage

```text
tut [OPTION]... FILE
```

Use `tut -- FILE` when the filename begins with `-`. `--help` and `--version` do not require a terminal.

TUT accepts regular files and symlinks to regular files. Input must be valid UTF-8 and no larger than 33,554,432 raw bytes. One leading UTF-8 BOM is ignored. LF, CRLF, and lone CR are displayed as line endings while their original bytes remain intact in memory. The source file is never written.

## Keys

| Key | Action |
|---|---|
| `j`, Down | Move down one visual row |
| `k`, Up | Move up one visual row |
| Space, Page Down, Ctrl-F | Move down one page |
| `b`, Page Up, Ctrl-B | Move up one page |
| Ctrl-D, Ctrl-U | Move half a page |
| `g`, Home | Go to the start |
| `G`, End | Go to the end |
| `/` | Edit a literal search |
| Enter, Escape | Commit or cancel search editing |
| `n`, `N` | Select the next or previous match |
| `q`, Ctrl-C | Quit |

Search is case-sensitive, literal, and byte-preserving. A search draft is limited to 4096 UTF-8 bytes. Backspace removes one extended grapheme cluster.

## Exit status

- `0` for help, version, normal quit, and keyboard Ctrl-C
- `1` for file, terminal, allocation, rendering, event, or restoration failure
- `2` for command-line usage errors
- `129`, `130`, or `143` for external SIGHUP, SIGINT, or SIGTERM after successful restoration

Diagnostics use the `tut: message` form on standard error. Terminal control characters in paths and diagnostics are escaped.

## Scope

Version 0.0.1 intentionally has no stdin document input, network access, configuration, persistence, plugins, Markdown semantics, or Web support. It uses the narrow Unicode terminal-width policy; rendering can still vary with terminal font and emulator behavior.

## Development

The current architecture uses absolute `u64` source-byte coordinates and an in-memory `DocumentStore` backend. See [docs/architecture.md](docs/architecture.md) for invariants and [docs/roadmap.md](docs/roadmap.md) for the path toward bounded paging, background search, and mature reader workflows.

## License

TUT is free software distributed under the MIT License. See [LICENSE](LICENSE).
