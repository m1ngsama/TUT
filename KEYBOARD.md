# TUT Browser - Keyboard Shortcuts

## 🎯 Quick Reference

### Navigation
| Key | Action |
|-----|--------|
| `o` | Open address bar (type URL and press Enter) |
| `Backspace` | Go back |
| `f` | Go forward |
| `r` or `F5` | Refresh current page |
| `q` or `Esc` or `F10` | Quit browser |

### Scrolling
| Key | Action |
|-----|--------|
| `j` or `↓` | Scroll down one line |
| `k` or `↑` | Scroll up one line |
| `Space` or `PageDown` | Page down |
| `b` or `PageUp` | Page up |
| `g` | Go to top |
| `G` | Go to bottom |

### Links
| Key | Action |
|-----|--------|
| `Tab` | Select next link |
| `Shift+Tab` | Select previous link |
| `1-9` | Jump to link by number |
| `Enter` | Follow selected link |

## 📝 Usage Examples

### Basic Browsing
```bash
./build_ftxui/tut https://example.com

# 1. Press 'j' or 'k' to scroll
# 2. Press 'Tab' to cycle through links
# 3. Press 'Enter' to follow the selected link
# 4. Press 'Backspace' to go back
# 5. Press 'q' to quit
```

### Direct Link Navigation
```bash
./build_ftxui/tut https://tldp.org/HOWTO/HOWTO-INDEX/howtos.html

# See numbered links like [1], [2], [3]...
# Press '1' to jump to first link
# Press '2' to jump to second link
# Press 'Enter' to follow the selected link
```

### Address Bar
```bash
# 1. Press 'o' to open address bar
# 2. Type new URL
# 3. Press 'Enter' to navigate
# 4. Press 'Esc' to cancel
```

## 🎨 UI Elements

### Top Bar
- `[◀]` - Back button (dimmed when can't go back)
- `[▶]` - Forward button (dimmed when can't go forward)
- `[⟳]` - Refresh
- Address bar - Shows current URL
- `[⚙]` - Settings (not yet implemented)
- `[?]` - Help (not yet implemented)

### Content Area
- Shows rendered HTML content
- Displays page title at top
- Shows scroll position at bottom

### Bottom Panels
- **Bookmarks Panel** - Shows bookmarks (not yet implemented)
- **Status Panel** - Shows:
  - Load stats (KB downloaded, time, link count)
  - Currently selected link URL

### Status Bar
- Shows function key shortcuts
- Shows current status message

## ⚡ Pro Tips

1. **Fast Navigation**: Use number keys (1-9) to instantly jump to links
2. **Quick Scrolling**: Use `Space` and `b` for fast page scrolling
3. **Link Preview**: Watch the status bar to see link URLs before following
4. **Efficient Browsing**: Use `g` to jump to top, `G` to jump to bottom
5. **Address Bar**: Type `o` quickly to enter a new URL

## 🐛 Known Limitations

- Ctrl+L not yet working for address bar (use 'o' instead)
- No search functionality yet (/ key)
- No bookmarks yet (Ctrl+D)
- No history panel yet (F3)

## 🚀 Coming Soon

- [ ] In-page search (`/` to search, `n`/`N` to navigate results)
- [ ] Bookmarks (add, remove, list)
- [ ] History (view and navigate)
- [ ] Better link highlighting
- [ ] Form support
