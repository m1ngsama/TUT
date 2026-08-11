# Security policy

## Supported code

Security fixes are developed on `main` and, when applicable, included in the
next signed release. The latest release is the supported distribution baseline;
older reader releases and historical terminal-browser tags do not receive
routine fixes.

TUT currently supports its documented single-session CLI use on Linux and
macOS. Behavior outside the documented input, platform, size, and process
lifecycle boundaries may still be relevant, but should not be assumed to have
the same support status.

## Private reporting

Do not open a public issue for a suspected vulnerability. Email
[contact@m1ng.space][report] and include:

- affected version or commit;
- operating system and terminal;
- whether input came from a path, standard input, or the terminal;
- impact and the smallest safe reproducer;
- whether the terminal, input file, or session log was left modified.

Please avoid including private document contents, search queries, credentials,
or unrelated terminal history. Reports involving terminal control sequences,
signals, paths, permissions, resource exhaustion, or cleanup behavior are in
scope.

The project will confirm the report, reproduce it where possible, and coordinate
disclosure after a fix is available. No fixed response-time guarantee is made.

[report]: mailto:contact@m1ng.space
