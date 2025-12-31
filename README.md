# TUT - Terminal UI Textual Browser

A lightweight, high-performance terminal browser with a btop-style interface.

![Version](https://img.shields.io/badge/version-0.1.0-blue)
![License](https://img.shields.io/badge/license-MIT-green)
![C++](https://img.shields.io/badge/C%2B%2B-17-orange)

## Features

- **btop-style UI** - Modern four-panel layout with rounded borders
- **Lightweight** - Binary size < 1MB, memory usage < 50MB
- **Fast startup** - Launch in < 500ms
- **Vim-style navigation** - j/k scrolling, / search, g/G jump
- **Keyboard-driven** - Full keyboard navigation with function key shortcuts
- **Themeable** - Multiple color themes (default, nord, gruvbox, solarized)
- **Configurable** - TOML-based configuration

## Screenshot

```
╭──────────────────────────────────────────────────────────────────────────────╮
│[◀] [▶] [⟳] ╭────────────────────────────────────────────────────────╮ [⚙] [?]│
│           │https://example.com                                      │        │
│           ╰────────────────────────────────────────────────────────╯        │
├──────────────────────────────────────────────────────────────────────────────┤
│                          Example Domain                                      │
├──────────────────────────────────────────────────────────────────────────────┤
│This domain is for use in illustrative examples in documents.                 │
│                                                                              │
│[1] More information...                                                       │
│                                                                              │
├────────────────────────────────────────┬─────────────────────────────────────┤
│📑 Bookmarks                            │📊 Status                            │
│  example.com                           │  ⬇ 1.2 KB  🕐 0.3s                  │
├────────────────────────────────────────┴─────────────────────────────────────┤
│[F1]Help [F2]Bookmarks [F3]History [F10]Quit                                  │
╰──────────────────────────────────────────────────────────────────────────────╯
```

## Installation

### Prerequisites

**macOS (Homebrew):**
```bash
brew install cmake gumbo-parser openssl ftxui cpp-httplib toml11
```

**Linux (Debian/Ubuntu):**
```bash
sudo apt install cmake libgumbo-dev libssl-dev
```

### Building from Source

```bash
git clone https://github.com/m1ngsama/TUT.git
cd TUT
cmake -B build -DCMAKE_PREFIX_PATH=/opt/homebrew  # macOS
cmake -B build                                      # Linux
cmake --build build -j$(nproc)
```

### Running

```bash
./build/tut                      # Start with blank page
./build/tut https://example.com  # Open URL directly
./build/tut --help               # Show help
```

## Keyboard Shortcuts

### Navigation
| Key | Action |
|-----|--------|
| `j` / `↓` | Scroll down |
| `k` / `↑` | Scroll up |
| `Space` | Page down |
| `b` | Page up |
| `g` | Go to top |
| `G` | Go to bottom |
| `Backspace` | Go back |
| `f` | Go forward |

### Links
| Key | Action |
|-----|--------|
| `Tab` | Next link |
| `Shift+Tab` | Previous link |
| `Enter` | Follow link |
| `1-9` | Jump to link by number |

### Search
| Key | Action |
|-----|--------|
| `/` | Start search |
| `n` | Next result |
| `N` | Previous result |

### UI
| Key | Action |
|-----|--------|
| `Ctrl+L` | Focus address bar |
| `F1` / `?` | Help |
| `F2` | Bookmarks |
| `F3` | History |
| `Ctrl+D` | Add bookmark |
| `Ctrl+Q` / `F10` / `q` | Quit |

## Configuration

Configuration files are stored in `~/.config/tut/`:

```
~/.config/tut/
├── config.toml      # Main configuration
└── themes/          # Custom themes
    └── mytheme.toml
```

### Example config.toml

```toml
[general]
theme = "default"
homepage = "https://example.com"
debug = false

[browser]
timeout = 30
user_agent = "TUT/0.1.0"

[ui]
word_wrap = true
show_images = true
```

## Project Structure

```
TUT/
├── CMakeLists.txt          # Build configuration
├── README.md               # This file
├── LICENSE                 # MIT License
├── cmake/                  # CMake modules
│   └── version.hpp.in
├── src/                    # Source code
│   ├── main.cpp           # Entry point
│   ├── core/              # Browser engine, HTTP, URL parsing
│   ├── ui/                # FTXUI components
│   ├── renderer/          # HTML rendering
│   └── utils/             # Logger, config, themes
├── tests/                  # Unit and integration tests
│   ├── unit/
│   └── integration/
└── assets/                 # Default configurations
    ├── config.toml
    ├── themes/
    └── keybindings/
```

## Dependencies

| Library | Purpose | Version |
|---------|---------|---------|
| [FTXUI](https://github.com/ArthurSonzogni/ftxui) | Terminal UI framework | 5.0+ |
| [cpp-httplib](https://github.com/yhirose/cpp-httplib) | HTTP client | 0.14+ |
| [gumbo-parser](https://github.com/google/gumbo-parser) | HTML parsing | 0.10+ |
| [toml11](https://github.com/ToruNiina/toml11) | TOML configuration | 3.8+ |
| [OpenSSL](https://www.openssl.org/) | HTTPS support | 1.1+ |

## Limitations

- **No JavaScript** - SPAs and dynamic content won't work
- **No CSS layout** - Only basic text formatting
- **No images** - ASCII art rendering planned for future
- **Text-only** - Focused on readable content

## Contributing

Contributions are welcome! Please read the coding style guidelines:

- C++17 standard
- Google C++ Style Guide
- Use `.hpp` for headers, `.cpp` for implementation
- All public APIs must have documentation comments

## License

MIT License - see [LICENSE](LICENSE) file for details.

## Authors

- **m1ngsama** - [GitHub](https://github.com/m1ngsama)

## Acknowledgments

- Inspired by [btop](https://github.com/aristocratos/btop) for UI design
- [FTXUI](https://github.com/ArthurSonzogni/ftxui) for the amazing TUI framework
- [lynx](https://lynx.invisible-island.net/) and [w3m](http://w3m.sourceforge.net/) for inspiration
