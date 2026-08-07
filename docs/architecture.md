# Architecture

TUT is one Rust package with a narrow dependency direction. `source` defines coordinates and borrowed source spans for `document`, `layout`, and `search`:

```text
source -+-> document -+
        +-> layout ----+-> app -> tui
        +-> search ----+
```

`cli` and `lib` orchestrate this pipeline without reversing its domain dependencies.

`error` is a leaf used by every layer. Domain modules do not depend on Crossterm or Ratatui. The TUI consumes immutable render state from `app` and does not open files, rewrap text, or run search.

## Data flow

1. `cli` parses `OsString` arguments without terminal or filesystem access.
2. `lib` validates both terminal streams and installs retained signal handlers.
3. `document` opens the path nonblocking, validates the opened handle, performs a bounded read, validates UTF-8, and places the original bytes in `DocumentStore::InMemory`.
4. `document` exposes an immutable `SourceText` view. It logically excludes one leading BOM but preserves its three-byte displacement.
5. `layout` projects extended grapheme clusters and raw line endings into terminal-safe atoms, then builds a compact vector of visual-row start offsets.
6. `search` builds a global non-overlapping literal-match bitset over the visible source span.
7. `app` owns navigation, resize anchoring, search transactions, and render-state construction.
8. `tui` maps events, writes frozen atoms directly to terminal cells, and owns terminal setup and restoration.

## Storage boundary

`Document` owns display metadata and a closed `DocumentStore` enum. The current `InMemory` variant owns one validated `String`; it does not rewrite BOM or line-ending bytes. `SourceText` is a small borrowed value containing `&str`, an absolute start, and an absolute end. Layout and search therefore depend on a source view rather than on storage ownership.

The next storage variant can provide bounded source windows without changing the meaning of offsets. Paging will require replacing whole-document consumers with explicit window requests, not changing persisted locations or inventing a second coordinate system.

## Coordinates and layout

Persistent positions are `SourceOffset(u64)` values measured from the beginning of the original file. A leading BOM makes the first content offset three rather than zero, and CRLF occupies two source bytes. Visual rows and display columns are derived state. Every visual-row start is a UTF-8 grapheme boundary and row starts are strictly increasing.

One projection policy is shared by wrapping and rendering:

- LF, CRLF, and lone CR are structural line endings.
- Tabs use four-column stops.
- C0, DEL, and C1 controls become one replacement cell.
- Standalone zero-width graphemes become one dotted-circle cell.
- Over-wide graphemes become one replacement cell until a wider reflow.
- Ordinary graphemes use the narrow `unicode-width` result.

The render layer receives the projected symbol and its approved cell width. It neither segments nor measures the symbol again.

## State invariants

`App` keeps an absolute source-byte anchor across width changes. `follow_end` is explicit, so a reader at the end remains at the end after reflow. Tiny terminals preserve logical state and accept only resize and quit.

Search editing is separate from the committed query. Committing performs one left-to-right non-overlapping scan. Match ranges use the same absolute source coordinates as layout, allowing a match to cross soft rows or partially intersect a grapheme while rendering highlights the entire visible grapheme.

## Terminal lifecycle

File validation and application construction complete before terminal mutation. The terminal session then enables raw mode, enters the alternate screen, and hides the cursor. Every attempted setup step marks its corresponding cleanup first.

Restoration always attempts show cursor, leave alternate screen, and disable raw mode in reverse order. The first restoration error is retained without suppressing later cleanup. A `Drop` implementation is an idempotent fallback. SIGHUP, SIGINT, and SIGTERM handlers record only the first signal, and the synchronous event loop checks that state around every blocking boundary.

## Resource policy

The 0.0.1 raw file limit is 32 MiB. Loading, UTF-8 validation, reflow, and search construction are linear in source bytes. Persistent layout stores only `u64` row starts, and persistent search stores one bit per readable source byte. Large derived allocations use fallible reservation. Rendering work is bounded by visible source bytes and visible match intersections.

## Unix and GNU conventions

TUT performs one focused job and keeps parsing, loading, layout, state, and terminal effects separate. It has predictable exit status, no hidden persistence, no network path, and no behavior based on the executable filename. The top-level Makefile exposes standard build, check, installation, uninstallation, and cleaning targets with `prefix`, `bindir`, and `DESTDIR` support. TUT follows GNU interface conventions but is not part of the GNU Project.
