# CI/CD Workflows - TEMPORARILY DISABLED

## Status: 🔴 Disabled During Active Development

The CI/CD workflows are currently disabled while we focus on core development.

### Why Disabled?

- The browser is in active development (v0.1.0-alpha)
- Core UI features are still being implemented
- Not ready for public releases yet
- Avoiding failed builds during rapid iteration

### What's Disabled?

- `release.yml.disabled` - Automated builds and releases

### When Will It Be Re-enabled?

Once we reach a stable state with:
- ✅ Core browsing functionality (DONE)
- ⏳ Interactive link navigation
- ⏳ Scrolling
- ⏳ Back/forward navigation
- ⏳ Basic bookmark/history support

### To Re-enable

Simply rename `release.yml.disabled` back to `release.yml`:
```bash
mv .github/workflows/release.yml.disabled .github/workflows/release.yml
```

### Current Development Focus

See [STATUS.md](../../STATUS.md) for current development status and roadmap.
