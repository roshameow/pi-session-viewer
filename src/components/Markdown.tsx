import React, { useEffect, useRef, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../api";
import { isLocalFileHref, localFileUrlTransform } from "../localFileLinks";

export type LocalFileLinkScope = {
  sessionPath: string;
  allowed: boolean;
  validate?: () => boolean;
  disabledReason?: string;
};

export const LocalFileLinkContext = React.createContext<LocalFileLinkScope>({
  sessionPath: "",
  allowed: false,
  disabledReason: "Local files are only available in a ready local session.",
});

type LinkMenu = {
  scope: LocalFileLinkScope;
  href: string;
  x: number;
  y: number;
  anchor: HTMLAnchorElement;
};

function MarkdownAnchor({ node, href, children, ...rest }: React.ComponentProps<"a"> & { node?: unknown }) {
  const scope = React.useContext(LocalFileLinkContext);
  const [failure, setFailure] = useState<{ scope: LocalFileLinkScope; href: string; message: string } | null>(null);
  const [menu, setMenu] = useState<LinkMenu | null>(null);
  const activeMenu = useRef<LinkMenu | null>(null);
  const menuElement = useRef<HTMLSpanElement | null>(null);
  const current = useRef({ scope, href, node, children, request: 0 });
  const requestChanged = current.current.scope !== scope || current.current.href !== href;
  if (requestChanged || current.current.node !== node || current.current.children !== children) {
    current.current = { scope, href, node, children, request: current.current.request + (requestChanged ? 1 : 0) };
    // Menus refer to a particular rendered link, but an already-submitted action
    // still deserves its error if streaming only reparses the SAME href/scope.
    activeMenu.current = null;
  }
  const mounted = useRef(true);
  const closeMenu = (restoreFocus = false) => {
    const previous = activeMenu.current;
    activeMenu.current = null;
    setMenu(null);
    if (restoreFocus) previous?.anchor.focus();
  };
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; activeMenu.current = null; current.current.request++; };
  }, []);

  const visibleMenu = menu && activeMenu.current === menu ? menu : null;
  useEffect(() => {
    const element = menuElement.current;
    if (!visibleMenu || !element) return;
    const bounds = element.getBoundingClientRect();
    const margin = 8;
    element.style.left = `${Math.max(margin, Math.min(visibleMenu.x, window.innerWidth - bounds.width - margin))}px`;
    element.style.top = `${Math.max(margin, Math.min(visibleMenu.y, window.innerHeight - bounds.height - margin))}px`;
    element.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
  }, [visibleMenu]);
  useEffect(() => {
    if (!visibleMenu) return;
    const outside = (event: PointerEvent) => {
      if (!menuElement.current?.contains(event.target as Node)) closeMenu();
    };
    const dismiss = () => closeMenu();
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); closeMenu(true); }
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("keydown", escape, true);
    window.addEventListener("scroll", dismiss, true);
    window.addEventListener("resize", dismiss);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("keydown", escape, true);
      window.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("resize", dismiss);
    };
  }, [visibleMenu]);

  if (!href || !isLocalFileHref(href)) {
    return <a {...rest} href={href || undefined} target="_blank" rel="noreferrer">{children}</a>;
  }

  const native = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  const unavailable = !scope.allowed || !scope.sessionPath
    ? scope.disabledReason || "Local files are unavailable for remote or not-ready sessions."
    : !native ? "Opening local files requires the desktop app; browser/demo mode cannot open them." : null;
  const sameScope = () => mounted.current && current.current.scope === scope && current.current.href === href;
  const sameLink = () => sameScope() && current.current.node === node && current.current.children === children;
  const act = async (action: "open" | "reveal") => {
    // Revalidate on action, including the source epoch before React rerenders.
    if (!sameLink()) return;
    const request = ++current.current.request;
    setFailure(null);
    if (scope.validate && !scope.validate()) {
      setFailure({ scope, href, message: "Session/source changed; choose a ready local session before opening this file." });
      return;
    }
    if (unavailable) {
      setFailure({ scope, href, message: unavailable });
      return;
    }
    try {
      // Action-only IPC. Only the backend resolves the actual session cwd.
      await (action === "open" ? api.openLocalFile : api.revealLocalFile)(href, scope.sessionPath);
    } catch (error) {
      if (sameScope() && current.current.request === request && (!scope.validate || scope.validate())) {
        setFailure({ scope, href, message: `Could not ${action} local file: ` + String(error) });
      }
    }
  };
  const showMenu = (anchor: HTMLAnchorElement, x: number, y: number) => {
    if (!sameLink() || (scope.validate && !scope.validate())) return;
    const next = { scope, href, x, y, anchor };
    activeMenu.current = next;
    setMenu(next);
  };
  const choose = (snapshot: LinkMenu, action: "open" | "reveal") => {
    if (activeMenu.current !== snapshot || !sameLink()) return;
    closeMenu(true);
    return act(action);
  };
  const menuKey = (event: React.KeyboardEvent<HTMLSpanElement>) => {
    if (event.key === "Tab") { closeMenu(); return; }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    event.stopPropagation();
    const items = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'));
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
      : (index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
    items[next]?.focus();
  };
  const error = failure?.scope === scope && failure.href === href ? failure.message : null;
  return <>
    <a {...rest} href={href} onClick={(event) => { event.preventDefault(); closeMenu(); return act("open"); }}
      onAuxClick={(event) => event.preventDefault()}
      onContextMenu={(event) => {
        event.preventDefault(); event.stopPropagation();
        showMenu(event.currentTarget, event.clientX, event.clientY);
      }}
      onKeyDown={(event) => {
        if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
        event.preventDefault(); event.stopPropagation();
        const bounds = event.currentTarget.getBoundingClientRect();
        showMenu(event.currentTarget, bounds.left, bounds.bottom);
      }}
      aria-haspopup="menu" aria-expanded={!!visibleMenu} aria-disabled={!!unavailable}
      title={unavailable || `Open local file (Markdown: configured app; HTML: default browser; scripts: forced text editor; other files: default app). Relative paths use this session's working directory: ${href}`}>
      {children}
    </a>
    {visibleMenu && <span ref={menuElement} role="menu" aria-label="本地文件" className="md-link-menu"
      style={{ left: visibleMenu.x, top: visibleMenu.y }} onKeyDown={menuKey}
      onContextMenu={(event) => { event.preventDefault(); event.stopPropagation(); }}>
      <button type="button" role="menuitem" tabIndex={-1} aria-disabled={!!unavailable} title={unavailable || undefined}
        onClick={() => choose(visibleMenu, "open")}>打开文件</button>
      <button type="button" role="menuitem" tabIndex={-1} aria-disabled={!!unavailable} title={unavailable || undefined}
        onClick={() => choose(visibleMenu, "reveal")}>在 Finder 中显示</button>
      {unavailable && <span className="md-link-menu-reason">{unavailable}</span>}
    </span>}
    {error && <span className="md-link-error" role="alert"> {error}</span>}
  </>;
}

// Stable renderer identities keep link state and memoized rows intact. Only
// anchors consume context, so a source change does not reparse Markdown.
const components: Components = {
  a: MarkdownAnchor,
  table: (p) => (
    <div className="md-table-wrap">
      <table {...p} />
    </div>
  ),
  code: ({ className, children, ...rest }) => {
    const isBlock = /language-/.test(className || "");
    if (isBlock) {
      return <code className="md-code-block" {...rest}>{children}</code>;
    }
    return <code className="md-code-inline" {...rest}>{children}</code>;
  },
};

export function Markdown({ text }: { text: string }) {
  // Unchanged text blocks skip react-markdown entirely via MemoMarkdown.
  return (
    <div className="md">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components} urlTransform={localFileUrlTransform}>
        {text}
      </ReactMarkdown>
    </div>
  );
}

export const MemoMarkdown = React.memo(Markdown);
