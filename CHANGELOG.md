# Changelog

## 0.1.1 — 2026-10-04

- Inventory native MCP configuration from session cwd and retain legacy adapter options, with separate source, enablement, and exposure semantics.
- Preserve native MCP / codemode / tool_search rendering and match nested / parallel tool results by call ID; show full expanded arguments.
- Separate live Pi identity from retained RMUX location (`ended rmux` / unknown / exited pane), preserve historical Attach targets, and remove automatic browsing cleanup.
- Align Pi CLI continuation with session cwd and drain stderr concurrently to avoid startup pipe backpressure and display diagnostics.
- Add offline React SSR and Rust adaptation regressions, including a bounded fake CLI test; live integration is not claimed.
- Synchronize package, lockfile, Cargo and Tauri application versions without changing dependencies; ignore private artifacts.

## Earlier source updates

- Add a browser preview using synthetic parent and child sessions and the shared UI components.
- Document source-build requirements and current platform scope.
- Add an unsigned local-build configuration without changing the maintainer's signing setup.
- Move signing, internals and historical performance observations into development notes.
