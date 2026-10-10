import type { Entry, SessionDetail } from "./types";

export type ThreadFilter = "default" | "no-tools" | "user-only" | "labeled-only" | "all";
export interface ThreadItem { entry: Entry; inlineResults: Entry[]; activeIdx: number }
const EMPTY_RESULTS: Entry[] = [];

export function entryText(e: Entry): string {
  const parts = [e.summary ?? "", e.name ?? "", e.label ?? ""];
  for (const c of e.content) {
    if (c.kind === "text") parts.push(c.text);
    else if (c.kind === "thinking") parts.push(c.thinking);
    else if (c.kind === "toolCall") parts.push(c.name + " " + c.arguments);
    else if (c.kind === "bash") parts.push(c.command + "\n" + c.output);
  }
  return parts.join("\n").toLowerCase();
}

// Two indexed passes, O(entries + calls + pairs). No per-result indexOf or
// forward scans. Bounded responses have already been searched/filtered across
// the entire branch by the backend; previews must never be re-filtered locally.
export function buildThreadItems(detail: SessionDetail, filter: ThreadFilter, query: string) {
  const active = detail.active.map(i => detail.entries[i]);
  const q = query.trim().toLowerCase();
  const labels = new Set(detail.entries.filter(e => e.kind === "label" && e.name).map(e => e.name));
  const passes = (e: Entry) => {
    if (detail.page) return true;
    if (q && !entryText(e).includes(q)) return false;
    const settings = ["label", "custom", "model_change", "thinking_level_change", "session_info"].includes(e.kind);
    if (filter === "user-only") return e.role === "user";
    if (filter === "labeled-only") return e.kind === "label" || e.labeled || labels.has(e.id);
    if (filter === "no-tools") return !settings && e.role !== "toolResult";
    return filter !== "default" || !settings;
  };
  const eligible = new Set(active.filter(passes).map(e => e.id));
  const byId = new Map(active.map(e => [e.id, e]));
  const callIdsByEntry = new Map<string, Set<string>>();
  for (const e of active) {
    if (e.role === "assistant") callIdsByEntry.set(e.id, new Set(e.content.flatMap(c => c.kind === "toolCall" ? [c.id] : [])));
  }
  const resultsByCall = new Map<string, Entry[]>();
  const paired = new Set<string>();
  const pairedToolsByCall = new Map<string, Set<string>>();
  const pair = (callEntryId: string, resultEntryId: string) => {
    const call = byId.get(callEntryId), result = byId.get(resultEntryId);
    // Leave previews as standalone rows so their explicit full-body control
    // cannot disappear inside a collapsed/filtered tool output.
    if (!call || !result || result.bodyRef?.preview || !eligible.has(call.id) || !eligible.has(result.id)) return;
    // A byte-budget preview may have no call block at all. Never hide a result
    // under an assistant that cannot render that specific tool row.
    if (!result.toolCallId || !callIdsByEntry.get(call.id)?.has(result.toolCallId)) return;
    const attached = pairedToolsByCall.get(call.id) ?? new Set<string>();
    if (attached.has(result.toolCallId)) return; // keep additional outputs standalone, never overwrite/hide them
    attached.add(result.toolCallId); pairedToolsByCall.set(call.id, attached);
    const list = resultsByCall.get(call.id) ?? [];
    list.push(result); resultsByCall.set(call.id, list); paired.add(result.id);
  };
  if (detail.page) {
    for (const p of detail.page.toolPairs) pair(p.callEntryId, p.resultEntryId);
  } else {
    const owners = new Map<string, string>();
    for (const e of active) {
      if (e.role === "assistant" || e.role === "user") owners.clear();
      if (e.role === "assistant") {
        for (const c of e.content) if (c.kind === "toolCall") owners.set(c.id, e.id);
      } else if (e.role === "toolResult" && e.toolCallId) {
        const owner = owners.get(e.toolCallId);
        if (owner) pair(owner, e.id);
      }
    }
  }
  const items: ThreadItem[] = [];
  active.forEach((entry, activeIdx) => {
    if (eligible.has(entry.id) && !paired.has(entry.id)) {
      items.push({ entry, activeIdx, inlineResults: resultsByCall.get(entry.id) ?? EMPTY_RESULTS });
    }
  });
  return { items, matchCount: detail.page?.matchedEntries ?? eligible.size, hideToolOutput: filter === "no-tools" };
}

// Prepending keeps existing Entry references and ID keys; mounted Markdown and
// expanded tool rows remain mounted. Cursor metadata describes the oldest page.
export function prependDetailPage(previous: SessionDetail, older: SessionDetail): SessionDetail {
  if (!previous.page || !older.page || previous.page.generation !== older.page.generation) {
    throw new Error("STALE_DETAIL: page generation changed");
  }
  const existing = new Map(previous.entries.map(e => [e.id, e]));
  const entries = older.entries.filter(e => !existing.has(e.id)).concat(previous.entries);
  const index = new Map(entries.map((e, i) => [e.id, i]));
  const ids = [...older.active.map(i => older.entries[i].id), ...previous.active.map(i => previous.entries[i].id)];
  const active = [...new Set(ids)].map(id => index.get(id)!);
  const pairs = new Map([...older.page.toolPairs, ...previous.page.toolPairs].map(p => [JSON.stringify(p), p]));
  return { ...previous, entries, active, page: { ...older.page, toolPairs: [...pairs.values()] } };
}
