# Track: packaging (P11)

- **Branch:** `rust/packaging`
- **Scope:** building, signing and releasing the native Nodus so it installs and runs like 0.9.4 on macOS (arm64, x64) and Windows (x64, arm64), and so every installed Electron copy (macOS 0.6.0+, Windows 0.9.0+) accepts it through its own updater (spec 06, HANDOFF CONTRACT; decisions D5, D9, D10, D11, D12).
- **Specs:** 06 (REL-001…REL-047), 01 (identity: bundle id, usage strings, AppUserModelID).

## Status

| Piece | State |
| --- | --- |
| macOS bundle (`packaging/macos/bundle.sh`) | ✅ arm64 built, ad-hoc signed and launched here |
| macOS DMG (`packaging/macos/dmg.sh`) | ✅ arm64 built here |
| macOS handoff gate (`packaging/macos/handoff-gate.sh`) | ✅ passes here (team check needs the real certificate) |
| Smoke test on the D11 driver (`packaging/smoke.py`) | ✅ passes here, run directly and through `open -n --env` |
