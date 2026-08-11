# TUT project principles

TUT is a terminal reader for finite, local, UTF-8 plain text. Its job is to
make text comfortable to read without compromising the terminal that hosts it.
It is intentionally a focused Unix tool rather than an editor, formatter,
terminal emulator, or general document platform.

These principles are part of the project's product contract. They guide both
implementation decisions and the acceptance of new features.

## Do one job well

TUT owns the interactive reading experience: safe terminal presentation,
navigation, search, position feedback, and predictable cleanup.

Content acquisition and transformation should normally remain composable. A
caller can decompress, filter, fetch, or convert content with another program
and pass the resulting finite UTF-8 stream to `tut -`. TUT does not execute
terminal control sequences embedded in the document and does not silently
become an editor or shell.

## Compose like a Unix tool

- A path names one input document; `-` reads standard input.
- Interactive commands come from the terminal even when content is piped.
- Diagnostics go to standard error and start with `tut:`.
- Informational output uses standard output.
- `--` terminates option processing.
- Exit status distinguishes success, invocation errors, runtime errors, and
  terminating signals.
- Optional session logs contain aggregate operational data, not document text,
  queries, paths, or individual keys.

TUT deliberately reads standard input to EOF before opening the interface. It
is a reader for finite snapshots, not a streaming filter or `tail -f` today.

## Preserve the terminal

Terminal ownership is a transaction. Raw mode, the alternate screen, cursor
visibility, signal dispositions, and job-control transitions must either be
established coherently or restored as completely as the operating system
allows.

Input text is data. Control and bidirectional formatting characters must not be
allowed to forge diagnostics or execute terminal behavior. Failures must be
reported explicitly; partially computed frames and search results must not be
published.

## Keep work bounded and interruptible

Every persistent structure and every potentially long foreground operation
needs a documented bound. Allocation must be fallible where hostile or maximum
input can reach it. Long work is divided into deterministic background steps so
that input and signals can be observed between steps.

An implementation is not complete merely because it eventually returns. It
must also define:

- maximum retained memory;
- maximum work and input consumed by one background step;
- cancellation and invalidation behavior;
- behavior when the source changes;
- the point at which a result becomes visible.

Runtime metrics and tests should prove these bounds without relying on wall
clock timing or a particular allocator.

## Treat text as text

Source positions are byte offsets, user-visible units are Unicode grapheme
clusters, and terminal layout is measured in cells. Code must not substitute
one coordinate system for another.

Invalid UTF-8 is rejected. Tabs, line endings, zero-width clusters, wide
clusters, and control characters have explicit display behavior. If an input
cannot be represented within a render budget, TUT prefers a safe replacement
or a typed error over ambiguous output.

## Keep interfaces stable

Command-line options, environment variables, exit behavior, key meanings, and
documented limits are public interfaces. Changes to them require tests,
documentation, and a changelog entry. Incompatible changes require an explicit
versioning decision rather than an accidental behavior drift.

`--help`, `--version`, the manual page, and the README are release artifacts,
not secondary commentary. A release is complete only when a user can discover,
build, install, operate, and remove the shipped program from those artifacts.

## Prefer evidence over assumptions

Correctness claims should be supported by deterministic unit or integration
tests. Terminal lifecycle and signal behavior require real PTY coverage on the
supported operating systems. Resource claims require exact counters and
boundary fixtures. Performance changes should preserve observable behavior and
include a regression test for the avoided work.

Linux and macOS are the currently tested platforms. Other Unix systems may
work, but support is not implied until their terminal and signal behavior is
covered.

## Admit features deliberately

Before adding a feature, answer all of the following:

1. Does it directly improve reading text in a terminal?
2. Would composition with an existing Unix tool solve the problem more clearly?
3. Does it preserve bounded memory, bounded background steps, and input/signal
   priority?
4. Are pipe, TTY, resize, suspension, cancellation, and error semantics clear?
5. Can its behavior and limits be tested deterministically?
6. Can it be documented without overstating the supported product scope?

A small coherent feature is preferable to a broad feature that weakens these
answers. Product quality is measured by predictability and usefulness, not by
the number of modes or options.
