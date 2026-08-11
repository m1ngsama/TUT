# Changelog

This file records user-visible changes to the current TUT terminal reader.

The repository contains older tags from an earlier terminal-browser project.
The reader's current version line begins with `v0.0.2`; the historical `v2.0.0`
tag does not describe the present program or its compatibility.

## Unreleased

### Added

- An in-application F1 help screen that can be opened while reading or entering
  a search and returns to the previous mode without losing the search draft.
- Static pending-screen feedback while the first view, a reflow, or a jump to
  the document end is being prepared.
- Responsive footer hints for 16-, 20-, 40-, and 80-column terminal layouts.
- Exact `TOP`, `END`, and `ALL` viewport boundary markers.
- Bounded numeric prefixes, up to 9999, for relative row, page, half-page, and
  search-match movement.
- Search-draft recall with Up and whole-draft clearing with Ctrl-U.

### Changed

- Long search drafts keep the most recently entered text visible.
- Adjacent viewport movement reuses validated cached row frontiers instead of
  repeatedly projecting the full screen.

## 0.0.4 - 2026-08-11

- Made viewport location, rendering, and visible-search highlighting
  incremental and preemptible.
- Bounded Unix terminal parser sequences and retained event state through a
  pinned crossterm maintenance fork.
- Added deterministic terminal EOF/HUP handling and restoration coverage.
- Hardened controlling-TTY input, nested-session rejection, job control, signal
  restoration, and input/log path separation.
- Added current and minimum-supported Rust CI, overflow-checked release tests,
  package verification, installation checks, and a verified signed release tag.

See the [v0.0.4 release notes][v0.0.4] for the complete release description.

## 0.0.3 - 2026-08-11

- Added finite standard-input snapshots while interactive commands remain on
  the controlling terminal.
- Added private typed session logs with fixed-size aggregate runtime summaries.
- Strengthened terminal restoration across quit, termination signals, and
  SIGTSTP/SIGCONT job control.
- Bounded terminal geometry, rendering, indexes, caches, and background work,
  while preserving Rust 1.88 as the minimum supported version.

See the [v0.0.3 release notes][v0.0.3] for the complete release description.

## 0.0.2 - 2026-08-10

- Rebuilt TUT as a local plain-text terminal reader with bounded positional
  file I/O and opened-file change detection.
- Added cooperative viewport location, search indexing, Unicode grapheme
  layout, terminal-control sanitization, and fair background scheduling.
- Added GNU-style command-line behavior and terminal/job-control restoration.
- Added source-package installation checks on Linux, alongside Linux and macOS
  test coverage.

See the [v0.0.2 release notes][v0.0.2] for the complete release description.

[v0.0.4]: https://github.com/m1ngsama/TUT/releases/tag/v0.0.4
[v0.0.3]: https://github.com/m1ngsama/TUT/releases/tag/v0.0.3
[v0.0.2]: https://github.com/m1ngsama/TUT/releases/tag/v0.0.2
