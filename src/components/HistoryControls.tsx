import type { DetailPage } from "../types";
import "./HistoryControls.css";

export interface HistoryControlsProps {
  page: DetailPage;
  loaded: number;
  busy: boolean;
  notice?: string | null;
  branchLeaf: string;
  activeBranchLeaf?: string;
  onBranchLeafChange: (value: string) => void;
  onLatest: () => void;
  onViewBranch: () => void;
  disabled?: boolean;
}

// Exact known neutral messages only: a new/unknown diagnostic must stay a warning.
const UPDATED_NOTICES = new Set([
  "History changed — refreshed bounded page. Previously loaded older pages/full bodies were released; load them again on demand.",
  "History changed — old cursor rejected. Refreshed latest bounded page; load older history again.",
]);
const SEARCHING_NOTICE = "Searching/filtering the full branch…";

/** Presentation only. The parent owns requests, filters and branch-reset semantics. */
export function HistoryControls({
  page, loaded, busy, notice, branchLeaf, activeBranchLeaf,
  onBranchLeafChange, onLatest, onViewBranch, disabled = false,
}: HistoryControlsProps) {
  const blocked = busy || disabled;
  const updated = !!notice && UPDATED_NOTICES.has(notice);
  const unknownNotice = !!notice && !updated && notice !== SEARCHING_NOTICE;
  const warnings = [
    ...(page.incompleteTail ? ["Incomplete trailing record — awaiting file update"] : []),
    ...(page.malformedLines > 0 ? [`${page.malformedLines} malformed lines skipped`] : []),
  ];
  const warning = unknownNotice || warnings.length > 0;
  const statusLabel = warning
    ? unknownNotice || warnings.length > 1 ? "History warning" : page.incompleteTail ? "Incomplete tail" : "Malformed lines"
    : updated ? "History updated" : null;
  const statusTitle = [...(notice ? [notice] : []), ...warnings].join("\n");
  const scope = activeBranchLeaf ? `selected branch ending at ${activeBranchLeaf}` : "current branch";

  return (
    <div className="history-controls" aria-label="History navigation" aria-busy={busy}>
      <span className="history-controls-count" title={`${loaded} loaded entries / ${page.matchedEntries} matching entries in the ${scope} (current search/filter)`}
        aria-label={`${loaded} loaded entries / ${page.matchedEntries} matching entries in the ${scope}`}>
        {loaded} / {page.matchedEntries}
      </span>
      {activeBranchLeaf && <span className="history-controls-branch" title={`Selected branch leaf: ${activeBranchLeaf}`}>
        Branch · {activeBranchLeaf}
      </span>}
      <span className="history-controls-status" role="status" aria-live="polite">
        {busy && <span className="history-controls-busy" title="Updating history results" aria-label="Updating history results">Updating…</span>}
        {statusLabel && <span className={`history-controls-chip${warning ? " history-controls-warning" : ""}`} title={statusTitle}>
          {warning && <span aria-hidden="true">⚠ </span>}{statusLabel}
        </span>}
      </span>
      <button type="button" className="history-controls-button" disabled={blocked}
        title={`Load latest matching entries in the ${scope}; keep current search/filter`}
        onClick={() => { if (!blocked) onLatest(); }}>Latest</button>
      <details className="history-controls-details" onKeyDown={event => {
        if (event.key === "Escape") {
          event.currentTarget.open = false;
          event.currentTarget.querySelector("summary")?.focus();
          event.stopPropagation();
        }
      }}>
        <summary className="history-controls-summary" title="History details and branch navigation" aria-label="History details and branch navigation">
          <span aria-hidden="true">⋯</span>
        </summary>
        <div className="history-controls-flyout">
          <div className="history-controls-heading">History details</div>
          <p className="history-controls-scope">{activeBranchLeaf ? <>Selected branch leaf: <code>{activeBranchLeaf}</code></> : "Current branch"} · matching count uses current search/filter; statistics cover this branch, not the whole file.</p>
          <p>{loaded} loaded / {page.matchedEntries} matching entries</p>
          <p>{page.branchEntries} branch entries / {page.totalEntries} file entries</p>
          <p>{page.counters.messages} messages · {page.counters.user} user · {page.counters.assistant} assistant · {page.counters.toolResult} tool results · {page.counters.labeled} labeled</p>
          <p className="history-controls-scope">{page.returnedEntries} entries in last fetched page · {page.payloadBytes} payload bytes · {page.hasMore ? "Older history available" : "No older matching entries"}</p>
          <p className="history-controls-scope">Generation: <code>{page.generation}</code></p>
          {notice && <p className={`history-controls-notice${unknownNotice ? " history-controls-warning" : ""}`}>{notice}</p>}
          {warnings.map(message => <p key={message} className="history-controls-notice history-controls-warning">{message}</p>)}
          <div className="history-controls-heading">Branch navigation</div>
          <p className="history-controls-scope">Leave empty to reset to the current branch. Latest stays in the selected branch.</p>
          <div className="history-controls-branch-form">
            <input aria-label="Branch leaf entry ID" placeholder="Branch leaf entry ID (optional)" value={branchLeaf} disabled={blocked}
              onChange={event => { if (!blocked) onBranchLeafChange(event.target.value); }} />
            <button type="button" className="history-controls-button" disabled={blocked}
              onClick={() => { if (!blocked) onViewBranch(); }}>View branch</button>
          </div>
        </div>
      </details>
    </div>
  );
}
