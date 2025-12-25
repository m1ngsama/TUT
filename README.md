TUT(1) - Terminal User Interface Browser
========================================

NAME
----
tut - vim-style terminal web browser

SYNOPSIS
--------
**tut** [*URL*]

**tut** **-h** | **--help**

DESCRIPTION
-----------
**tut** is a text-mode web browser designed for comfortable reading in the
terminal. It extracts and displays the textual content of web pages with a
clean, centered layout optimized for reading, while providing vim-style
keyboard navigation.

The browser does not execute JavaScript or display images. It is designed
for reading static HTML content, documentation, and text-heavy websites.

OPTIONS
-------
*URL*
    Open the specified URL on startup. If omitted, displays the built-in
    help page.

**-h**, **--help**
    Display usage information and exit.

KEYBINDINGS
-----------
**tut** uses vim-style keybindings throughout.

### Navigation

**j**, **Down**
    Scroll down one line.

**k**, **Up**
    Scroll up one line.

**Ctrl-D**, **Space**
    Scroll down one page.

**Ctrl-U**, **b**
    Scroll up one page.

**gg**
    Jump to top of page.

**G**
    Jump to bottom of page.

**[***count***]G**
    Jump to line *count* (e.g., **50G** jumps to line 50).

**[***count***]j**, **[***count***]k**
    Scroll down/up *count* lines (e.g., **5j** scrolls down 5 lines).

### Link Navigation

**Tab**
    Move to next link.

**Shift-Tab**, **T**
    Move to previous link.

**Enter**
    Follow current link.

**h**, **Left**
    Go back in history.

**l**, **Right**
    Go forward in history.

### Search

**/**
    Start search. Enter search term and press **Enter**.

**n**
    Jump to next search match.

**N**
    Jump to previous search match.

### Marks

**m***[a-z]*
    Set mark at current position (e.g., **ma**, **mb**).

**'***[a-z]*
    Jump to mark (e.g., **'a**, **'b**).

### Mouse

**Left Click**
    Click on links to follow them directly.

**Scroll Wheel Up/Down**
    Scroll page up or down.

Works with most modern terminal emulators that support mouse events.

### Commands

Press **:** to enter command mode. Available commands:

**:q**, **:quit**
    Quit the browser.

**:o** *URL*, **:open** *URL*
    Open *URL*.

**:r**, **:refresh**
    Reload current page.

**:h**, **:help**
    Display help page.

**:***number*
    Jump to line *number*.

### Other

**r**
    Reload current page.

**q**
    Quit the browser.

**?**
    Display help page.

**ESC**
    Cancel command or search input.

LIMITATIONS
-----------
**tut** does not execute JavaScript. Modern single-page applications (SPAs)
built with React, Vue, Angular, or similar frameworks will not work correctly,
as they require JavaScript to render content.

To determine if a site will work with **tut**, use:

    curl https://example.com | less

If you can see the actual content in the HTML source, the site will work.
If you only see JavaScript code or empty div elements, it will not.

Additionally:
- No image display
- No CSS layout support
- No AJAX or dynamic content loading

EXAMPLES
--------
View the built-in help:

    tut

Browse Hacker News:

    tut https://news.ycombinator.com

Read Wikipedia:

    tut https://en.wikipedia.org/wiki/Unix_philosophy

Open a URL, search for "unix", and navigate:

    tut https://example.com
    /unix<Enter>
    n

DEPENDENCIES
------------
- ncurses or ncursesw (for terminal UI)
- libcurl (for HTTPS support)
- CMake >= 3.15 (build time)
- C++17 compiler (build time)

INSTALLATION
------------
### From Source

**macOS (Homebrew):**

    brew install cmake ncurses curl
    mkdir -p build && cd build
    cmake ..
    cmake --build .
    sudo install -m 755 tut /usr/local/bin/

**Linux (Debian/Ubuntu):**

    sudo apt-get install cmake libncursesw5-dev libcurl4-openssl-dev
    mkdir -p build && cd build
    cmake ..
    cmake --build .
    sudo install -m 755 tut /usr/local/bin/

**Linux (Fedora/RHEL):**

    sudo dnf install cmake gcc-c++ ncurses-devel libcurl-devel
    mkdir -p build && cd build
    cmake ..
    cmake --build .
    sudo install -m 755 tut /usr/local/bin/

### Using Makefile

    make
    sudo make install

FILES
-----
No configuration files are used. The browser is stateless and does not
store history, cookies, or cache.

ENVIRONMENT
-----------
**tut** respects the following environment variables:

**TERM**
    Terminal type. Must support basic cursor movement and colors.

**LINES**, **COLUMNS**
    Terminal size. Automatically detected via ncurses.

EXIT STATUS
-----------
**0**
    Success.

**1**
    Error occurred (e.g., invalid URL, network error, ncurses initialization
    failure).

PHILOSOPHY
----------
**tut** follows the Unix philosophy:

1. Do one thing well: display and navigate text content from the web.
2. Work with other programs: output can be piped, URLs can come from stdin.
3. Simple and minimal: no configuration files, no persistent state.
4. Text-focused: everything is text, processed and displayed cleanly.

The design emphasizes keyboard efficiency, clean output, and staying out
of your way.

SEE ALSO
--------
lynx(1), w3m(1), curl(1), vim(1)

BUGS
----
Report bugs at: https://github.com/m1ngsama/TUT/issues

AUTHORS
-------
m1ngsama <contact@m1ng.space>

Inspired by lynx, w3m, and vim.

LICENSE
-------
MIT License. See LICENSE file for details.
