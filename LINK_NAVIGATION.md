# Quick Link Navigation Guide

The browser now supports vim-style quick navigation to links!

## Features

### 1. Visual Link Numbers
All links are displayed inline in the text with numbers like `[0]`, `[1]`, `[2]`, etc.
- Links are shown with yellow color and underline
- Active link (selected with Tab) has yellow background

### 2. Quick Navigation Methods

#### Method 1: Number + Enter
Type a number and press Enter to jump to that link:
```
3<Enter>     - Jump to link [3]
10<Enter>    - Jump to link [10]
```

#### Method 2: 'f' command (follow)
Press `f` followed by a number to immediately open that link:
```
f3           - Open link [3] directly
f10          - Open link [10] directly
```

Or type the number first:
```
3f           - Open link [3] directly
10f          - Open link [10] directly
```

#### Method 3: Traditional Tab navigation (still works)
```
Tab          - Next link
Shift-Tab/T  - Previous link
Enter        - Follow current highlighted link
```

## Examples

Given a page with these links:
- "Google[0]"
- "GitHub[1]"
- "Wikipedia[2]"

You can:
- Press `1<Enter>` to select GitHub link
- Press `f2` to immediately open Wikipedia
- Press `Tab` twice then `Enter` to open Wikipedia

## Usage

```bash
# Test with a real website
./tut https://example.com

# View help
./tut
# Press ? for help
```

## Key Bindings Summary

| Command | Action |
|---------|--------|
| `[N]<Enter>` | Jump to link N |
| `f[N]` or `[N]f` | Open link N directly |
| `Tab` | Next link |
| `Shift-Tab` / `T` | Previous link |
| `Enter` | Follow current link |
| `h` | Go back |
| `l` | Go forward |

All standard vim navigation keys (j/k, gg/G, /, n/N) still work as before!
