# TUT Browser Testing Guide

This document provides comprehensive testing instructions to ensure the browser works correctly.

## Quick Start

```bash
# Build the browser
cmake -B build -S . -DCMAKE_BUILD_TYPE=Debug
cmake --build build

# Run with a test site
./build/tut http://example.com

# Or use the interactive test script
./test_browser.sh
```

## Feature Testing Checklist

### Basic Navigation
- [ ] Browser loads and displays page correctly
- [ ] Scroll with j/k works smoothly
- [ ] Page up/down (Ctrl+d/u) works
- [ ] Go to top (gg) and bottom (G) works
- [ ] Back (h) and forward (l) navigation works

### Link Navigation
- [ ] Tab cycles through links
- [ ] Shift+Tab goes to previous link
- [ ] Enter follows the active link
- [ ] Links are highlighted when active
- [ ] Link URLs shown in status bar

### Search
- [ ] Press / to enter search mode
- [ ] Type search term and press Enter
- [ ] Matches are highlighted
- [ ] n/N navigate between matches
- [ ] Search count shown in status bar

### Form Interaction
- [ ] Press 'i' to focus first form field
- [ ] Tab/Shift+Tab navigate between fields
- [ ] Enter on text input enters edit mode
- [ ] Text can be typed and edited
- [ ] Backspace removes characters
- [ ] Enter or Esc exits edit mode
- [ ] Checkbox toggles with Enter
- [ ] SELECT dropdown shows options
- [ ] j/k navigate dropdown options
- [ ] Enter selects option in dropdown
- [ ] Selected option displays correctly

### Bookmarks
- [ ] Press B to bookmark current page
- [ ] Press D to remove bookmark
- [ ] Type :bookmarks to view all bookmarks
- [ ] Bookmarks persist between sessions
- [ ] Can click bookmarks to open pages

### History
- [ ] Type :history to view browsing history
- [ ] History shows URLs and titles
- [ ] History entries are clickable
- [ ] History persists between sessions
- [ ] Recent pages appear at top

### Performance
- [ ] Page loads are async with spinner
- [ ] Esc cancels page loading
- [ ] Page cache works (revisit loads instantly)
- [ ] Image cache works (images load from cache)
- [ ] Status shows "cached: N" for cached images
- [ ] Scrolling is smooth
- [ ] No noticeable lag in UI

### Commands
- [ ] :o URL opens new URL
- [ ] :q quits the browser
- [ ] :bookmarks shows bookmarks
- [ ] :history shows history
- [ ] :help shows help page
- [ ] ? also shows help page

### Edge Cases
- [ ] Window resize updates layout correctly
- [ ] Very long pages scroll correctly
- [ ] Pages without links/forms work
- [ ] Unicode text displays correctly
- [ ] CJK characters display correctly
- [ ] Images render as ASCII art (if stb_image available)
- [ ] Error handling for failed page loads

## Test Websites

### Simple Test Sites
1. **http://example.com** - Basic HTML test
2. **http://info.cern.ch** - First website ever, very simple
3. **http://motherfuckingwebsite.com** - Minimalist design
4. **http://textfiles.com** - Text-only content

### Form Testing
Create a local test file (test_form.html is provided):
```bash
python3 -m http.server 8000
./build/tut http://localhost:8000/test_form.html
```

Test the form features:
- Text input editing
- Checkbox toggling
- Dropdown selection
- Tab navigation

### Performance Testing
1. Load a page
2. Press 'r' to refresh (should use cache)
3. Load the same page again (should be instant from cache)
4. Check status bar shows "cached" messages

## Known Limitations

- HTTPS support depends on libcurl configuration
- Some complex JavaScript-heavy sites won't work (static HTML only)
- File:// URLs may not work depending on curl configuration
- Form submission is not yet implemented
- Cookies are not yet supported

## Reporting Issues

When reporting issues, please include:
1. The URL you were trying to load
2. The exact steps to reproduce
3. Expected vs actual behavior
4. Any error messages

## Success Criteria

The browser is working correctly if:
1. ✓ Can load and display simple HTML pages
2. ✓ Navigation (scroll, links) works smoothly
3. ✓ Form interaction is responsive and intuitive
4. ✓ Bookmarks and history persist correctly
5. ✓ Caching improves performance noticeably
6. ✓ No crashes or hangs during normal use
