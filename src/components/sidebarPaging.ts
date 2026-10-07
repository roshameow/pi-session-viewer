import { useState } from "react";
import type { SessionMeta } from "../types";

export const SESSION_PAGE_SIZE = 100;

// Scope-derived defaults apply during the first render after a project/source/
// search change, not in an effect after potentially rendering the old huge page.
export function useScopedState<T>(scope: string, initial: () => T) {
  const [stored, setStored] = useState(() => ({ scope, value: initial() }));
  const value = stored.scope === scope ? stored.value : initial();
  // Record each transition too: going back to the previous query/project must
  // not resurrect its large page. React retries this component before commit.
  if (stored.scope !== scope) setStored({ scope, value });
  const update = (next: T | ((previous: T) => T)) => {
    setStored((previous) => {
      const current = previous.scope === scope ? previous.value : initial();
      return {
        scope,
        value: typeof next === "function" ? (next as (previous: T) => T)(current) : next,
      };
    });
  };
  return [value, update] as const;
}

// Keep ordering and all metadata intact. A selected row beyond the page is an
// extra pinned row, rather than mounting every row preceding it.
export function sessionPage(sessions: SessionMeta[], limit: number, selectedPath: string | null) {
  const rows = sessions.slice(0, limit);
  if (selectedPath) {
    const index = sessions.findIndex((s) => s.path === selectedPath);
    if (index >= limit) rows.push(sessions[index]);
  }
  return rows;
}
