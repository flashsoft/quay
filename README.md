# Quay

A macOS desktop wrapper for `frpc`, built with Tauri v2.

## Release

Quay uses a macOS-only GitHub Actions release workflow.

### Trigger

Push a Git tag matching `v*`, for example:

```bash
git tag v0.1.0
git push origin v0.1.0
```

### What the workflow does

- runs on `macos-latest`
- installs Node.js, pnpm, and Rust
- installs frontend dependencies
- builds the Tauri app
- creates or updates a GitHub Release
- uploads macOS artifacts

### Published assets

- `Quay_*.dmg` — primary download for end users
- `Quay_*.app.tar.gz` — app bundle asset for debugging or manual inspection

### Workflow file

- `.github/workflows/release.yml`

### Notes

- current release flow is **unsigned** and **not notarized**
- users may still see Gatekeeper prompts on first launch
- signing and notarization secrets are intentionally only reserved in the workflow comments for future setup
