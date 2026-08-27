# 260605_phase0-fork-rebrand

## Objective
Fork the brew-browser scaffold into a new app, "Agency Agents", as the foundation for an
app-store-for-AI-agents over the agency-agents repo.

## Outcome
- ✅ `cargo check` green · `vite build` green
- ✅ Rebranded: productName "Agency Agents", id `app.rubezhanin.agency-agents-app`, window title,
  CSP (re-pointed to GitHub hosts), Cargo/lib/main, app.html, keychain service id.

## Files Modified
- `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/src/main.rs`,
  `src/app.html`, `src-tauri/src/github/auth.rs` (KEYCHAIN_SERVICE + its 2 guard tests).
- `src-tauri/data/*` — restored brew's real bundled data so brew-domain tests stay green until
  that domain retires.

## Notes
- Reference clones kept read-only: `/tmp/brew-browser-inspect`, `/tmp/agency-agents-inspect`.
- Methodology + architecture captured in `memory-bank/{projectbrief,systemPatterns,contracts,decisions}.md`.
