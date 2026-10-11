import React, { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { AuthorizedDirectory, AuthorizedDirectoryPage, ImportedDataFile } from "../types";

const PAGE_SIZE = 200;

// A bounded instruction about ONE staged file; sourcePath is deliberately never used.
export function copiedFileInstruction(file: ImportedDataFile): string | null {
  if (!file.path || file.path.length > 4096 || /[\u0000-\u001f\u007f]/.test(file.path)) return null;
  return `仅使用已复制到本地应用私有存储的这一个文件：${JSON.stringify(file.path)}。不要读取原始 App 数据目录或尝试绕过系统授权。先说明拟进行的分析；未经我明确同意，不要通过工具把文件内容发送给模型或其他服务。`;
}

interface Props {
  onClose: () => void;
  // Omitted unless a selected local detail exists and is idle. App rechecks its
  // captured source/detail epoch at the click, including stale rendered buttons.
  onInsertDraft?: (instruction: string) => boolean;
  targetName?: string;
}
interface Browser {
  id: string;
  relativePath: string;
  offset: number;
  page: AuthorizedDirectoryPage | null;
  loading: boolean;
}

export function DataAccessPanel({ onClose, onInsertDraft, targetName }: Props) {
  const [directories, setDirectories] = useState<AuthorizedDirectory[]>([]);
  const [copies, setCopies] = useState<ImportedDataFile[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [browser, setBrowser] = useState<Browser | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const life = useRef(0);
  const mounted = useRef(false);
  const operation = useRef<string | null>(null);
  const directoryIds = useRef(new Set<string>());
  const browseEpoch = useRef(0);
  const dialog = useRef<HTMLDivElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const current = (epoch: number) => mounted.current && life.current === epoch;

  const loadMetadata = async (epoch: number) => {
    if (!current(epoch) || operation.current) return;
    operation.current = "metadata";
    setLoading(true);
    setError(null);
    ++browseEpoch.current;
    setBrowser(null);
    // These APIs only read persisted receipts. Never enumerate source folders here.
    const results = await Promise.allSettled([
      Promise.resolve().then(() => api.listAuthorizedDirectories()),
      Promise.resolve().then(() => api.listImportedDataFiles()),
    ]);
    if (!current(epoch)) return;
    const [dirs, files] = results;
    const errors: string[] = [];
    if (dirs.status === "fulfilled") {
      directoryIds.current = new Set(dirs.value.map(d => d.id));
      setDirectories(dirs.value);
    } else errors.push(`无法读取授权记录：${String(dirs.reason)}`);
    if (files.status === "fulfilled") setCopies(files.value);
    else errors.push(`无法读取副本记录：${String(files.reason)}`);
    setError(errors.length ? errors.join("；") : null);
    operation.current = null;
    setLoading(false);
  };

  useEffect(() => {
    mounted.current = true;
    const epoch = ++life.current;
    operation.current = null;
    void loadMetadata(epoch);
    const previous = document.activeElement as HTMLElement | null;
    closeButton.current?.focus();
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); onClose(); }
      if (event.key !== "Tab") return;
      const controls = dialog.current?.querySelectorAll<HTMLElement>("button:not(:disabled), [href], input:not(:disabled), [tabindex='0']");
      if (!controls?.length) return;
      const first = controls[0], last = controls[controls.length - 1];
      if (event.shiftKey && (document.activeElement === first || !dialog.current?.contains(document.activeElement))) {
        event.preventDefault(); last.focus();
      } else if (!event.shiftKey && (document.activeElement === last || !dialog.current?.contains(document.activeElement))) {
        event.preventDefault(); first.focus();
      }
    };
    document.addEventListener("keydown", keydown);
    return () => {
      mounted.current = false;
      ++life.current;
      ++browseEpoch.current;
      document.removeEventListener("keydown", keydown);
      previous?.focus?.();
    };
  }, [onClose]);

  const mutate = async (key: string, work: (epoch: number) => Promise<void>) => {
    if (!mounted.current || operation.current) return;
    const epoch = life.current;
    operation.current = key; // synchronous double-click guard
    setBusy(key);
    setError(null);
    setNotice(null);
    try { await work(epoch); }
    catch (e) { if (current(epoch)) setError(String(e)); }
    finally {
      if (current(epoch)) { operation.current = null; setBusy(null); }
    }
  };

  const choose = () => mutate("choose", async epoch => {
    const row = await api.chooseAuthorizedDirectory();
    if (!current(epoch)) return;
    if (!row) { setNotice("已取消选择，未添加授权。"); return; }
    directoryIds.current.add(row.id);
    setDirectories(prev => [...prev.filter(d => d.id !== row.id), row]);
    setNotice("已记住此目录授权，供查看器只读读取入口后续复用。此操作没有浏览、复制或上传文件。点击 Browse 才读取一层目录。");
  });

  const revoke = (row: AuthorizedDirectory) => mutate(`revoke:${row.id}`, async epoch => {
    if (!directoryIds.current.has(row.id)) return;
    // Invalidate at the START of revocation, not after IPC returns. An in-flight
    // browse must not resurrect revoked rows, even if deletion fails or is slow.
    ++browseEpoch.current;
    setBrowser(null);
    directoryIds.current.delete(row.id);
    try { await api.removeAuthorizedDirectory(row.id); }
    catch (e) {
      if (current(epoch)) directoryIds.current.add(row.id);
      throw e;
    }
    if (!current(epoch)) return;
    setDirectories(prev => prev.filter(d => d.id !== row.id));
    setNotice("已撤销本应用的目录记录。已有副本不会删除；系统级授权不会因此撤销。");
  });

  const browse = async (id: string, relativePath = "", offset = 0) => {
    if (!mounted.current || operation.current || !directoryIds.current.has(id)) return;
    const epoch = life.current;
    const request = ++browseEpoch.current;
    const valid = () => current(epoch) && browseEpoch.current === request && directoryIds.current.has(id);
    setError(null);
    setNotice(null);
    setBrowser({ id, relativePath, offset, page: null, loading: true });
    try {
      const page = await api.authorizedDirectoryEntries(id, relativePath, offset);
      if (!valid()) return;
      setBrowser({ id, relativePath: page.relativePath, offset, page, loading: false });
    } catch (e) {
      if (!valid()) return;
      setError(`无法浏览目录：${String(e)}`);
      setBrowser({ id, relativePath, offset, page: null, loading: false });
    }
  };

  const copyFile = (id: string, relativePath: string) => mutate("copy", async epoch => {
    if (!directoryIds.current.has(id)) return;
    const file = await api.importAuthorizedFile(id, relativePath);
    if (!current(epoch)) return;
    setCopies(prev => [file, ...prev.filter(f => f.id !== file.id)]);
    setNotice("已复制这一个文件到本地应用私有存储。未发送到会话或模型；副本将保留到你删除为止。");
  });

  const deleteCopy = (file: ImportedDataFile) => {
    if (operation.current || !window.confirm(`删除本地副本 ${file.name}？\n${file.path}\n仅删除副本，不修改原文件；已插入的会话草稿不会自动移除。`)) return;
    return mutate(`delete:${file.id}`, async epoch => {
      await api.removeImportedDataFile(file.id);
      if (!current(epoch)) return;
      setCopies(prev => prev.filter(f => f.id !== file.id));
      setNotice("本地副本已删除。原文件未修改；如曾插入草稿，请自行移除已失效的路径。");
    });
  };

  const insert = (file: ImportedDataFile) => {
    if (!mounted.current || operation.current) return;
    const instruction = copiedFileInstruction(file);
    if (!instruction || !onInsertDraft) return;
    if (onInsertDraft(instruction)) {
      setNotice("已插入当前本地会话草稿，未自动发送。请关闭面板检查草稿；通过工具发送文件内容可能暴露给配置的模型。");
    } else setError("会话或来源已改变，或正在运行。未插入；请选择空闲的本地会话后重试。");
  };

  const disabled = loading || busy !== null;
  const dir = directories.find(d => d.id === browser?.id);
  return (
    <div className="data-access-backdrop" onClick={e => { if (e.target === e.currentTarget) onClose(); }}>
      <div ref={dialog} className="data-access-panel" role="dialog" aria-modal="true" aria-labelledby="data-access-title" aria-describedby="data-access-privacy">
        <header className="data-access-header">
          <h2 id="data-access-title">Data access / 数据目录</h2>
          <button ref={closeButton} type="button" onClick={onClose} aria-label="关闭数据目录">关闭 ×</button>
        </header>
        <div className="data-access-content">
          <div id="data-access-privacy" className="data-access-privacy">
            <p><strong>所选数据可能包含私人微信消息。</strong>仅授权目录不会传输或上传任何数据；本面板不自动把目录列表或提示注入 Pi。</p>
            <p>来源访问代理只读。仅在你点击单个文件的复制按钮时复制到本地应用私有存储，副本保留到手动删除。撤销目录授权不删除副本，也不撤销操作系统授予的权限。</p>
            <p>这不是任务沙箱，也不阻止其他访问方式：直接用 Python / Pi 访问原始路径仍可能触发系统授权提示。发送文件内容（包括通过工具）可能暴露给当前配置的模型，请先检查再发送。</p>
          </div>
          {error && <div className="data-access-error" role="alert">{error}</div>}
          {notice && <p role="status">{notice}</p>}
          {(loading || busy) && <p role="status">{loading ? "正在读取授权和副本记录（不扫描来源）…" : busy === "choose" ? "等待系统目录选择器…" : "操作处理中…"} 可关闭面板；已开始的系统操作可能仍会完成，重新打开可查看记录。</p>}
          <section aria-labelledby="authorized-title">
            <h3 id="authorized-title">已记住授权的本地目录</h3>
            <p id="data-access-remember-consent">点击「选择目录并记住授权」并在系统选择器中确认，表示同意本应用保存所选目录的授权，供以后通过查看器只读读取入口复用；可随时撤销本应用保存的授权。</p>
            <p className="data-access-muted">保存的是 security-scoped bookmark（授权书签）。系统可恢复时复用；恢复失败时显示错误，请重新选择。当前 App 未启用 App Sandbox，重启后访问受保护微信目录是否免提示仍需验证。不保证永久免提示，也不是对 Python / Pi 直接访问的「始终允许」。</p>
            <div className="data-access-actions">
              <button type="button" className="data-access-primary" aria-describedby="data-access-remember-consent" disabled={disabled} onClick={() => void choose()}>选择目录并记住授权</button>
              <button type="button" disabled={disabled} onClick={() => void loadMetadata(life.current)}>刷新记录 / 重试</button>
            </div>
            {!loading && directories.length === 0 && <p className="data-access-muted">暂无记住的目录授权。请明确选择所需目录；记住授权后不会自动扫描。</p>}
            {directories.map(row => <article className="data-access-card" key={row.id}>
              <strong>{row.name}</strong>
              <div className="data-access-muted">已保存授权凭据 · 查看器读取入口</div>
              <div className="data-access-path">{row.path}</div>
              <div className="data-access-muted">授权时间：{new Date(row.createdAt).toLocaleString()}</div>
              <div className="data-access-actions">
                <button type="button" disabled={disabled} onClick={() => void browse(row.id)}>Browse / 浏览</button>
                <button type="button" disabled={disabled} onClick={() => void revoke(row)}>撤销目录授权</button>
              </div>
            </article>)}
          </section>
          {browser && dir && <section className="data-access-browser" aria-labelledby="browse-title" aria-busy={browser.loading}>
            <h3 id="browse-title">只读浏览：{dir.name}</h3>
            <div className="data-access-path">相对目录：{browser.relativePath || "/（授权根目录）"}</div>
            <p className="data-access-muted">仅显示当前一层，每页最多 200 项。不递归扫描；不能批量复制或写入来源。目录页不是快照，文件变化时请重新读取，页间可能变化。</p>
            <div className="data-access-actions">
              <button type="button" disabled={disabled || browser.loading || !browser.relativePath} onClick={() => void browse(browser.id, browser.relativePath.split("/").slice(0, -1).join("/"))}>上一级</button>
              <button type="button" disabled={disabled || browser.loading} onClick={() => void browse(browser.id, browser.relativePath, browser.offset)}>重新读取此页</button>
              <button type="button" onClick={() => { ++browseEpoch.current; setBrowser(null); }}>关闭浏览</button>
            </div>
            {browser.loading ? <p role="status">读取当前目录页…</p> : browser.page && <>
              {browser.page.entries.length === 0 && <p>此页没有条目。</p>}
              <ul className="data-access-entries">
                {browser.page.entries.map(entry => <li key={entry.relativePath}>
                  <div><span>{entry.isDirectory ? "📁 " : "文件 "}{entry.name}</span>{!entry.isDirectory && <small>{entry.size.toLocaleString()} bytes</small>}</div>
                  {entry.isDirectory
                    ? <button type="button" disabled={disabled} onClick={() => void browse(browser.id, entry.relativePath)}>打开文件夹</button>
                    : <button type="button" disabled={disabled} onClick={() => void copyFile(browser.id, entry.relativePath)}>仅复制此文件</button>}
                </li>)}
              </ul>
              <div className="data-access-actions">
                <button type="button" disabled={disabled || browser.offset === 0} onClick={() => void browse(browser.id, browser.relativePath, Math.max(0, browser.offset - PAGE_SIZE))}>上一页</button>
                <span>第 {Math.floor(browser.offset / PAGE_SIZE) + 1} 页</span>
                <button type="button" disabled={disabled || !browser.page.hasMore} onClick={() => void browse(browser.id, browser.relativePath, browser.offset + PAGE_SIZE)}>下一页</button>
              </div>
            </>}
          </section>}
          <section aria-labelledby="copies-title">
            <h3 id="copies-title">本地副本 / Import receipts</h3>
            <p className="data-access-muted">以下文件位于应用私有存储。未自动发送；目录撤销后副本仍保留。{onInsertDraft ? `插入目标：${targetName ?? "当前本地会话"}` : "选择非运行中的本地会话才可插入草稿；远程来源不可用。"}</p>
            {!loading && copies.length === 0 && <p>暂无副本。</p>}
            {copies.map(file => <article className="data-access-card" key={file.id}>
              <strong>{file.name}</strong>
              <div className="data-access-path">副本路径：{file.path}</div>
              <div>大小：{file.size.toLocaleString()} bytes</div>
              <div className="data-access-path data-access-muted">来源路径：{file.sourcePath}</div>
              <div className="data-access-actions">
                {onInsertDraft && <button type="button" disabled={disabled || !copiedFileInstruction(file)} onClick={() => insert(file)}>插入当前会话草稿</button>}
                <button type="button" disabled={disabled} onClick={() => void deleteCopy(file)}>删除副本…</button>
              </div>
            </article>)}
          </section>
        </div>
      </div>
    </div>
  );
}
