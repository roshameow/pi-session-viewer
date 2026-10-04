# Changelog

## 0.1.2 — 2026-10-04

- Infer yellow busy from verified Pi identity plus the last pending user / assistant toolUse / toolResult transcript message, not live SDK `isStreaming`; long thinking / tools no longer expire solely at 60 seconds without JSONL writes. Final assistant stop / error / abort stays idle.
- Keep UNKNOWN identity as a short fresh-transcript weak fallback; an explicitly dead Pi / retained shell stays false.
- Recognize anchored modern `pi-subagent-task-*` titles while rejecting shell mentions; skip normal `pi_subagent_exit(exitCode=0)` metadata when finding `agent_settled`.
- Capture remote host time alongside process snapshots, use fixed snapshot mtime for legacy age guards, and include modern worker titles in sync filtering. Remote state is still the latest manual sync, not real-time.
- Add five pure running regressions and an opt-in, project-scoped read-only inventory. Coverage does not guarantee auto-retry, compaction, oversized JSON, SSH degradation, or GUI / remote end-to-end behavior.
- Synchronize application / root lockfile versions to 0.1.2 without dependency changes; retain the existing development signing identity.

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
