# Roadmap

TUT is evolving from a reliable small-file reader into a mature terminal reader. Each phase must remain useful on its own, preserve terminal restoration, support Rust 1.88.0, and keep the build and command interface compatible with GNU conventions.

## 1. Stable source model

Status: implemented.

- Use `SourceOffset(u64)` as the only persistent byte coordinate.
- Retain original UTF-8 and line-ending bytes in `DocumentStore::InMemory`.
- Treat a leading BOM as excluded content while preserving absolute offsets.
- Make layout, reflow anchors, search ranges, and highlights share source coordinates.
- Test source bases above `u32::MAX` without allocating multi-gigabyte input.

## 2. Bounded paging

- Add a Unix `read_at` backend without changing the source-coordinate contract.
- Build sparse physical-line checkpoints incrementally.
- Keep a bounded cache around the viewport and reuse buffers.
- Decode UTF-8 and grapheme boundaries across page edges without replacement or loss.
- Remove the fixed whole-file limit only after memory and latency budgets are enforced by tests and benchmarks.

The reader should request the smallest useful source window. Storage should not know about terminal rows, and layout should not own file descriptors.

## 3. Responsive background work

- Keep `App` and terminal mutation on the UI thread.
- Use bounded standard-library channels for indexing and search results.
- Tag work with document generations so stale results are discarded deterministically.
- Express cancellation as generation replacement rather than shared mutable flags where possible.
- Avoid `Arc<Mutex<App>>`; immutable requests and owned results should cross thread boundaries.

## 4. Reader workflows

- Add go-to-line and go-to-percent commands on top of the sparse index.
- Add explicit reload and follow modes with inode and metadata checks.
- Add bookmarks and reading-position persistence through an optional XDG state file.
- Support `tut -` by reading document bytes from standard input and terminal events from `/dev/tty`.
- Keep every side effect opt-in, inspectable, and removable.

## 5. Text quality and hardening

- Introduce Unicode line-breaking only behind a measurable layout contract.
- Handle suspend and resume without weakening restoration guarantees.
- Add property tests for wrapping, source ranges, and page boundaries.
- Add fuzz targets for UTF-8 windows, grapheme projection, and search indexing.
- Run Miri on pure domain modules and retain bounded PTY lifecycle tests.

## Release discipline

Every increment must pass formatting, warnings-as-errors Clippy, stable tests, Rust 1.88.0 tests, release builds, staged installation checks, and an English-only source and documentation audit. New dependencies require a concrete reduction in code or risk.
