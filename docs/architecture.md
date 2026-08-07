# Architecture

TUT is one Rust package with a narrow dependency direction:

```text
cli      document      layout      search
                 \       |       /
                         app
                          |
                         tui
```

`error` is a leaf used by every layer. Domain modules do not depend on Crossterm or Ratatui. The TUI consumes immutable render state from `app` and does not open files, rewrap text, or run search.

## Data flow

1. `cli` parses `OsString` arguments without terminal or filesystem access.
2. `lib` validates both terminal streams and installs retained signal handlers.
3. `document` opens the path nonblocking, validates the opened handle, performs a bounded read, validates UTF-8, and normalizes into one immutable `String`.
4. `layout` projects extended grapheme clusters into terminal-safe atoms and builds a compact vector of visual-row start offsets.
5. `search` builds a global non-overlapping literal-match bitset.
6. `app` owns navigation, resize anchoring, search transactions, and render-state construction.
7. `tui` maps events, writes frozen atoms directly to terminal cells, and owns terminal setup and restoration.

## Coordinates and layout

Persistent positions are `u32` byte offsets into normalized UTF-8. Visual rows and display columns are derived state. Every visual-row start is a grapheme boundary and row starts are strictly increasing.

One projection policy is shared by wrapping and rendering:

- LF is structural.
- Tabs use four-column stops.
- C0, DEL, and C1 controls become one replacement cell.
- Standalone zero-width graphemes become one dotted-circle cell.
- Over-wide graphemes become one replacement cell until a wider reflow.
- Ordinary graphemes use the narrow `unicode-width` result.

The render layer receives the projected symbol and its approved cell width. It neither segments nor measures the symbol again.

## State invariants

`App` keeps a normalized-byte anchor across width changes. `follow_end` is explicit, so a reader at the end remains at the end after reflow. Tiny terminals preserve logical state and accept only resize and quit.

Search editing is separate from the committed query. Committing performs one left-to-right non-overlapping scan. Match ranges use the same normalized byte coordinates as layout, allowing a match to cross soft rows or partially intersect a grapheme while rendering highlights the entire visible grapheme.

## Terminal lifecycle

File validation and application construction complete before terminal mutation. The terminal session then enables raw mode, enters the alternate screen, and hides the cursor. Every attempted setup step marks its corresponding cleanup first.

Restoration always attempts show cursor, leave alternate screen, and disable raw mode in reverse order. The first restoration error is retained without suppressing later cleanup. A `Drop` implementation is an idempotent fallback. SIGHUP, SIGINT, and SIGTERM handlers record only the first signal, and the synchronous event loop checks that state around every blocking boundary.

## Resource policy

The 0.0.1 raw file limit is 32 MiB. Loading, normalization, reflow, and search construction are linear in source bytes. Persistent layout stores only row starts, and persistent search stores one bit per normalized source byte. Large derived allocations use fallible reservation. Rendering work is bounded by visible source bytes and visible match intersections.

## Unix and GNU conventions

TUT performs one focused job and keeps parsing, loading, layout, state, and terminal effects separate. It has predictable exit status, no hidden persistence, no network path, and no behavior based on the executable filename. The top-level Makefile exposes standard build, check, installation, uninstallation, and cleaning targets with `prefix`, `bindir`, and `DESTDIR` support. TUT follows GNU interface conventions but is not part of the GNU Project.
