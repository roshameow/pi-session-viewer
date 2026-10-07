// Identity is an epoch, not just a name: A -> B -> A must reject the first A.
export type RequestScope = "source" | "project" | "detail";
export interface RequestToken {
  scope: RequestScope;
  source: string | null;
  project: string | null;
  path: string | null;
  sourceEpoch: number;
  projectEpoch: number;
  detailEpoch: number;
}

export class RequestSequencer {
  source: string | null = null;
  project: string | null = null;
  path: string | null = null;
  committedSource: string | null = null;
  ready = false;
  private sourceEpoch = 0;
  private projectEpoch = 0;
  private detailEpoch = 0;
  private inFlight = new Map<string, Promise<unknown>>();

  capture(scope: RequestScope = "source"): RequestToken {
    return { scope, source: this.source, project: this.project, path: this.path,
      sourceEpoch: this.sourceEpoch, projectEpoch: this.projectEpoch, detailEpoch: this.detailEpoch };
  }

  current(token: RequestToken, allowPending = false): boolean {
    return (allowPending || this.ready) && token.sourceEpoch === this.sourceEpoch &&
      (token.scope === "source" || token.projectEpoch === this.projectEpoch) &&
      (token.scope !== "detail" || token.detailEpoch === this.detailEpoch);
  }

  beginSource(host: string | null): RequestToken {
    this.source = host;
    this.project = this.path = null;
    this.sourceEpoch++;
    this.projectEpoch++;
    this.detailEpoch++;
    this.ready = false;
    return this.capture();
  }

  adoptSource(token: RequestToken, host: string | null): boolean {
    if (!this.current(token, true)) return false;
    this.source = this.committedSource = host;
    this.ready = true;
    return true;
  }

  // Backend selection epochs guard commits. Do not queue a cached/local
  // selection behind a potentially long initial sync for an obsolete source.
  async commitSource(token: RequestToken, commit: () => Promise<void>): Promise<boolean> {
    if (!this.current(token, true)) return false;
    await commit();
    return this.adoptSource(token, token.source);
  }

  selectProject(key: string | null): void {
    if (this.project === key) return;
    this.project = key;
    this.path = null;
    this.projectEpoch++;
    this.detailEpoch++;
  }

  selectDetail(path: string | null): void {
    if (this.path === path) return;
    this.path = path;
    this.detailEpoch++;
  }

  // Reuses only an equivalent request in the same epoch. Never shares the
  // first A's pending request with a later A after a source/project/detail hop.
  request<T>(token: RequestToken, key: string, run: () => Promise<T>): Promise<T> {
    const identity = JSON.stringify([token.scope, token.sourceEpoch,
      token.scope === "source" ? 0 : token.projectEpoch,
      token.scope === "detail" ? token.detailEpoch : 0, key]);
    const existing = this.inFlight.get(identity);
    if (existing) return existing as Promise<T>;
    const result = Promise.resolve().then(() => {
      if (!this.current(token, true)) throw new Error("Superseded request");
      return run();
    });
    this.inFlight.set(identity, result);
    const clear = () => { if (this.inFlight.get(identity) === result) this.inFlight.delete(identity); };
    result.then(clear, clear);
    return result;
  }
}

// Two frames, then a task: cached content can be committed and painted before
// invoking a potentially expensive remote refresh. Both handles are cancellable.
export function afterViewPaint(run: () => void): () => void {
  let frame = requestAnimationFrame(() => {
    frame = requestAnimationFrame(() => { timer = window.setTimeout(run, 0); });
  });
  let timer: number | undefined;
  return () => {
    cancelAnimationFrame(frame);
    if (timer !== undefined) window.clearTimeout(timer);
  };
}
