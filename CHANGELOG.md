# Changelog

## 0.1.6 — 2026-10-10

- Ordinary detail selection/polling uses a generation-bound active-branch page: 100 entries and at most 256 KiB serialized response. Full branch filters/search/counters, old-page cursors, entry/branch navigation and tool peers remain available.
- Oversized entries are visibly incomplete previews; explicit UTF-8-aligned chunks preserve the original JSONL record, with cancellation/download/release. No silent code/evidence truncation or automatic giant-body hydration.
- Read-only streaming offsets/metadata replace retained full-message indexes. Arc index/parsed caches have 16 MiB/4 MiB estimated byte budgets; full-detail deep-clone cache is removed. Terminal/attach/send CWD paths use header-only lookup. Explicit whole-session export remains independent of downloaded pages.
- Linear tool pairing, debounced server search, lazy collapsed arguments and stable entry keys reduce frontend allocation/remount work. Source/detail epochs and changed-history refresh safety remain intact.
- First cold index still scans the file. Fixture/backend timings are not native GUI paint/typing guarantees; no session rewrite, history deletion or model/gate change.

## 0.1.5 — 2026-10-07

- Bound initial Sidebar DOM with 100-row progressive pages and explicit show-more; search still covers all loaded sessions, total counts/grouping/parent labels remain intact and off-page selection stays visible.
- Retain source/project-qualified session results in an eight-entry LRU with the original 2s freshness/fingerprint and desktop-running invalidation. Coalesce equivalent cold inventory fills and share request evidence rather than repeatedly cloning/probing per row.
- Guard source/project/detail replies with request generations, coalesce equivalent requests and stop canceled polls from publishing stale sessions, errors or completion toasts. Preserve source-qualified unsent drafts.
- Select usable remote cached history before background refresh; expose initial-sync readiness, cached timestamp, refresh phase/error and non-live-snapshot status. Bound/coalesce refresh and restrict sync to managed browsing files; preserve excluded cache files and existing SSH configuration.
- Pin each traversal to its source. Cache refresh is not a transactionally atomic history/runtime snapshot; partial failure is reported without claiming freshness or whole-tree completeness.
- Large directories can still take seconds on first cold load. Cache/backend fixtures are not installed GUI latency guarantees; no history pruning, worker/process termination or network-route changes.

## 0.1.4 — 2026-10-07

- Index Sidebar parent lookups instead of repeated per-worker session scans; retain nested worker labels and direct-main grouping.
- Read session headers incrementally within the original byte/line limits; collect full-transcript legacy parent calls only when an unmarked worker needs the fallback. Conflicting explicit lineage never invokes guessing.
- Reuse parsed bounded discovery preambles per file, and retain incremental legacy-parent caches for eight source-qualified projects; unchanged A→B→A switches no longer rescan A transcripts.
- Validate local runtime PID slots with one batched process lookup instead of per-slot subprocesses; preserve PID-reuse and private SDK owner checks.
- Read remote runtime/terminal/RMUX state exclusively from its captured source snapshots, never desktop process/TTY/CWD lookups. Scope result caches to their source and snapshot fingerprints; missing/corrupt evidence remains unknown.
- Keep source-build/development signing and the stable terminal helper unchanged. Remote state is still the latest synchronized snapshot, not live telemetry; no history pruning or forced Pi/worker cleanup.

## 0.1.3 — 2026-10-07

- Resolve explicit nested subagent parents against all canonical sessions, so a grandchild points to its worker parent rather than showing “no parent”. Keep real-session preference over mirrors.
- Leave missing, self/cyclic, or ambiguous explicit lineage unresolved without guessing; retain legacy text matching only when no explicit marker exists. Parent labels support workers; sidebar grouping remains direct children of main sessions.

- Synchronize application/package versions to 0.1.3 without dependency updates; retain the existing signing identity and stable terminal helper.

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
