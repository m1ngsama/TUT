# Contributing to TUT

Thank you for helping improve TUT. The project values small, reviewable changes
with explicit behavior and resource bounds.

Before proposing a feature, read the [project principles](docs/PROJECT.md).
Features should improve terminal reading directly, compose cleanly with Unix
tools, and preserve input and signal responsiveness.

## Development environment

TUT is currently developed and tested on Linux and macOS. The minimum supported
Rust version is recorded as `rust-version` in `Cargo.toml`; `rust-toolchain.toml`
pins the current development toolchain.

Clone the repository and run:

```sh
cargo test --all-targets --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo fmt --all -- --check
```

Before requesting review, also run the release and distribution checks when the
change can affect arithmetic, packaging, installation, or terminal behavior:

```sh
RUSTFLAGS='-C overflow-checks=on' cargo test --release --all-targets --locked
make distcheck
```

To verify the minimum supported compiler when it is installed:

```sh
cargo +1.88.0 clippy --all-targets --all-features --locked -- -D warnings
cargo +1.88.0 test --all-targets --locked
```

## Change requirements

- Preserve public CLI, key, exit-status, environment, and documented-limit
  behavior unless the change explicitly updates that contract.
- Add deterministic tests for externally visible behavior and regression fixes.
- Use real PTY tests for terminal lifecycle, signals, job control, or controlling
  terminal behavior; mocks alone are not sufficient for those interfaces.
- Give every new retained structure and long-running operation an explicit
  memory/work bound and cancellation story.
- Keep partial render, navigation, index, and search results private until they
  are complete and validated.
- Update `README.md`, `docs/tut.1`, and `CHANGELOG.md` when their documented
  behavior changes.
- Avoid timing assertions when exact counters, state transitions, or bounded
  test drivers can prove the property instead.

Do not commit build output, temporary worktrees, generated release archives, or
local session logs.

## Reporting problems

Use the [issue tracker](https://github.com/m1ngsama/TUT/issues) for reproducible
bugs and focused feature proposals. Include the TUT version, operating system,
terminal, input type (path or standard input), and the smallest safe reproducer.

Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).
